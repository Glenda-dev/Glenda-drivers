mod config;
mod consts;
#[cfg(feature = "unicode")]
mod utf8;

use crate::layout::SHM_VA;
use config::*;
use consts::*;
use glenda::cap::{Frame, IrqHandler};
use glenda::drivers::interface::UartDriver;
use glenda::error::Error;
use glenda::io::uring::IoUringServer;
use glenda::mem::shm::SharedMemory;
#[cfg(feature = "unicode")]
use utf8::Utf8Decoder;

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    #[cfg(feature = "unicode")]
    decoder: Utf8Decoder,
    pub ring: Option<IoUringServer>,
    pub shm: Option<SharedMemory>,
    pub pending_read: Option<glenda::io::uring::IoUringSqe>,
}

impl Ns16550a {
    pub fn new(base: usize, irq: IrqHandler) -> Self {
        Self {
            base,
            irq,
            #[cfg(feature = "unicode")]
            decoder: Utf8Decoder::new(),
            ring: None,
            shm: None,
            pending_read: None,
        }
    }

    pub fn set_ring_server(&mut self, ring: IoUringServer) {
        self.ring = Some(ring);
    }

    pub fn setup_shm(
        &mut self,
        frame: Frame,
        client_vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> Result<(), Error> {
        let mut shm = SharedMemory::new(frame, SHM_VA, size);
        shm.set_client_vaddr(client_vaddr);
        shm.set_paddr(paddr);
        self.shm = Some(shm);
        Ok(())
    }

    pub unsafe fn read_reg(&self, offset: usize) -> u8 {
        let ptr = (self.base + offset) as *const u8;
        core::ptr::read_volatile(ptr)
    }

    pub unsafe fn write_reg(&self, offset: usize, val: u8) {
        let ptr = (self.base + offset) as *mut u8;
        core::ptr::write_volatile(ptr, val);
    }

    pub fn init_hw(&self) {
        self.set_baud_rate(DEFAULT_BAUD_RATE);
        unsafe {
            // Enable RX interrupt
            self.write_reg(IER, IER_RX_ENABLE);
        }
    }

    pub fn set_baud_rate(&self, baud: u32) {
        let divisor = calculate_divisor(baud);
        unsafe {
            // Disable interrupts during init
            self.write_reg(IER, 0x00);

            // Enable DLAB to set baud rate
            self.write_reg(LCR, LCR_DLAB);

            // Set divisor
            self.write_reg(DLL, (divisor & 0xFF) as u8);
            self.write_reg(DLM, (divisor >> 8) as u8);

            // 8 bits, no parity, one stop bit (8N1), disable DLAB
            self.write_reg(LCR, LCR_DATA_BITS_8 | LCR_STOP_BITS_1 | LCR_PARITY_NONE);

            // Enable FIFO, clear them, with 14-byte threshold
            self.write_reg(FCR, FCR_FIFO_ENABLE | FCR_FIFO_RX_RESET | FCR_FIFO_TX_RESET);

            // IRQs enabled, RTS/DTR set
            self.write_reg(MCR, MCR_OUT2 | MCR_RTS | MCR_DTR);
        }
    }

    pub fn handle_irq(&mut self) -> Result<(), Error> {
        let mut count = 0;
        let mut bytes = [0u8; 16];

        loop {
            unsafe {
                let iir = self.read_reg(IIR);
                if iir & IIR_NO_INTERRUPT != 0 {
                    break;
                }

                if let Some(c) = self.getchar() {
                    if count < bytes.len() {
                        bytes[count] = c;
                        count += 1;
                    }
                } else {
                    break;
                }
            }
        }

        if count > 0 {
            if let Some(sqe) = self.pending_read.take() {
                let addr = sqe.addr;
                let len = core::cmp::min(sqe.len as usize, count);
                let user_data = sqe.user_data;

                if let Some(shm) = &self.shm {
                    let client_vaddr = shm.client_vaddr();
                    if (addr as usize) >= client_vaddr
                        && (addr as usize) + len <= client_vaddr + shm.size()
                    {
                        let server_vaddr = shm.vaddr() + (addr as usize - client_vaddr);
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                server_vaddr as *mut u8,
                                len,
                            );
                        }
                    } else {
                        error!(
                            "NS16550A IRQ error: Read address {:#x} (len {}) out of SHM boundary",
                            addr, len
                        );
                        if let Some(mut ring) = self.ring.take() {
                            let _ = ring.complete(user_data, -(Error::InvalidArgs as i32));
                            self.ring = Some(ring);
                        }
                        return Ok(());
                    }
                } else {
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, len);
                    }
                }

                if let Some(mut ring) = self.ring.take() {
                    let _ = ring.complete(user_data, len as i32);
                    self.ring = Some(ring);
                }
            }
        }
        Ok(())
    }

    pub fn handle_sq(&mut self) {
        if let Some(mut ring) = self.ring.take() {
            while let Some(sqe) = ring.next_request() {
                match sqe.opcode {
                    glenda::io::uring::IOURING_OP_WRITE => {
                        let addr = sqe.addr;
                        let len = sqe.len as usize;
                        let user_data = sqe.user_data;

                        // Use SHM if available, otherwise assume linear mapping (not safe but compatible)
                        if let Some(shm) = &self.shm {
                            // Convert client-side addr to server-side vaddr
                            let client_vaddr = shm.client_vaddr();
                            let server_vaddr_base = shm.vaddr();

                            if (addr as usize) < client_vaddr
                                || (addr as usize) + len > client_vaddr + shm.size()
                            {
                                error!("NS16550A error: Write address {:#x} (len {}) out of SHM boundary", addr, len);
                                let _ = ring.complete(user_data, -(Error::InvalidArgs as i32));
                                continue;
                            }

                            let server_vaddr = server_vaddr_base + (addr as usize - client_vaddr);
                            let buf = unsafe {
                                core::slice::from_raw_parts(server_vaddr as *const u8, len)
                            };
                            for &b in buf {
                                self.putchar(b);
                            }
                        }
                        let _ = ring.complete(user_data, len as i32);
                    }
                    glenda::io::uring::IOURING_OP_READ => {
                        // For UART, READ SQEs could be queued and completed when data arrives.
                        let addr = sqe.addr;
                        let user_data = sqe.user_data;

                        let mut count = 0;
                        if let Some(c) = self.getchar() {
                            if let Some(shm) = &self.shm {
                                let client_vaddr = shm.client_vaddr();
                                if (addr as usize) >= client_vaddr
                                    && (addr as usize) < client_vaddr + shm.size()
                                {
                                    let server_vaddr = shm.vaddr() + (addr as usize - client_vaddr);
                                    unsafe { *(server_vaddr as *mut u8) = c };
                                    count = 1;
                                } else {
                                    error!(
                                        "NS16550A error: Read address {:#x} out of SHM boundary",
                                        addr
                                    );
                                    let _ = ring.complete(user_data, -(Error::InvalidArgs as i32));
                                    continue;
                                }
                            }
                        }

                        if count > 0 {
                            let _ = ring.complete(user_data, count as i32);
                        } else {
                            // If no data, we store the SQE and complete it in handle_irq.
                            // Ensure any previous pending read is not overwritten if it hasn't been completed.
                            if self.pending_read.is_none() {
                                self.pending_read = Some(sqe);
                            }
                        }
                    }
                    _ => {
                        let _ = ring.complete(sqe.user_data, -(Error::NotSupported as i32));
                    }
                }
            }
            self.ring = Some(ring);
        }
    }

    pub fn handle_cq(&mut self) {
        // CQ notification from client means client has consumed some entries.
        // For a simple UART driver, we might not need to do anything here
        // unless we were flow-controlled.
    }

    fn process_char(&mut self, _c: char) {
        // Obsolete: Driver is now data-agnostic.
    }

    fn put_unicode_char(&self, c: char) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        for b in s.as_bytes() {
            self.putchar(*b);
        }
    }

    fn getchar(&self) -> Option<u8> {
        unsafe {
            if self.read_reg(LSR) & LSR_DATA_READY != 0 {
                Some(self.read_reg(RBR))
            } else {
                None
            }
        }
    }

    fn putchar(&self, c: u8) {
        unsafe {
            while self.read_reg(LSR) & LSR_THR_EMPTY == 0 {}
            self.write_reg(THR, c);
        }
    }
}

impl UartDriver for Ns16550a {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if let Ok(s) = core::str::from_utf8(buf) {
            log!("UART Write Sync ({} bytes): {}", buf.len(), s);
        } else {
            log!("UART Write Sync ({} bytes): {:?}", buf.len(), buf);
        }
        Ok(buf.len())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut count = 0;
        while count < buf.len() {
            if let Some(c) = self.getchar() {
                buf[count] = c;
                count += 1;
            } else {
                break;
            }
        }
        Ok(count)
    }

    fn set_baud_rate(&mut self, baud: u32) {
        Self::set_baud_rate(self, baud);
    }
}

impl core::fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = self.write(s.as_bytes());
        Ok(())
    }
}
