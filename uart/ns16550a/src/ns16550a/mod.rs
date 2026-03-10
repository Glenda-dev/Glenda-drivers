mod config;
mod consts;

use crate::layout::SHM_VA;
use alloc::collections::VecDeque;
use config::*;
use consts::*;
use glenda::cap::{Frame, IrqHandler};
use glenda::drivers::interface::UartDriver;
use glenda::error::Error;
use glenda::io::uring::IoUringServer;
use glenda::mem::shm::SharedMemory;

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    pub ring: Option<IoUringServer>,
    pub shm: Option<SharedMemory>,
    pub pending_read: Option<glenda::io::uring::IoUringSqe>,
    pub rx_buffer: VecDeque<u8>,
}

impl Ns16550a {
    pub fn new(base: usize, irq: IrqHandler) -> Self {
        Self {
            base,
            irq,
            ring: None,
            shm: None,
            pending_read: None,
            rx_buffer: VecDeque::with_capacity(1024),
        }
    }

    pub fn set_ring_server(&mut self, ring: IoUringServer) {
        self.ring = Some(ring);
    }

    pub fn setup_shm(
        &mut self,
        frame: Frame,
        client_vaddr: usize,
        paddr: usize,
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
        let mut has_data = false;
        loop {
            unsafe {
                let iir = self.read_reg(IIR);
                if iir & IIR_NO_INTERRUPT != 0 {
                    break;
                }

                let id = iir & IIR_ID_MASK;
                if id == IIR_RX_DATA_READY || id == IIR_TIMEOUT || id == IIR_RLS {
                    while let Some(c) = self.getchar() {
                        if self.rx_buffer.len() < 1024 {
                            self.rx_buffer.push_back(c);
                            has_data = true;
                        }
                    }
                }
            }
        }

        if has_data {
            self.try_complete_read();
        }
        Ok(())
    }

    fn try_complete_read(&mut self) {
        if self.rx_buffer.is_empty() {
            return;
        }

        if let Some(sqe) = self.pending_read.take() {
            let addr = sqe.addr;
            let len = core::cmp::min(sqe.len as usize, self.rx_buffer.len());
            let user_data = sqe.user_data;

            if let Some(shm) = &self.shm {
                let client_vaddr = shm.client_vaddr();
                if (addr as usize) >= client_vaddr
                    && (addr as usize) + len <= client_vaddr + shm.size()
                {
                    let server_vaddr = shm.vaddr() + (addr as usize - client_vaddr);
                    unsafe {
                        let buf = core::slice::from_raw_parts_mut(server_vaddr as *mut u8, len);
                        for i in 0..len {
                            buf[i] = self.rx_buffer.pop_front().unwrap();
                        }
                        // Ensure data is written to memory before CQE is pushed
                        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
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
                    return;
                }
            } else {
                unsafe {
                    let buf = core::slice::from_raw_parts_mut(addr as *mut u8, len);
                    for i in 0..len {
                        buf[i] = self.rx_buffer.pop_front().unwrap();
                    }
                }
            }

            if let Some(mut ring) = self.ring.take() {
                let _ = ring.complete(user_data, len as i32);
                self.ring = Some(ring);
            }
        }
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
                                // Ensure all data writes from client are visible before we read
                                core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
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
                        if self.pending_read.is_none() {
                            self.pending_read = Some(sqe);
                            // Critical: try to complete immediately if rx_buffer has data
                            self.try_complete_read();
                        } else {
                            error!("NS16550A: Multiple concurrent reads not supported yet.");
                            let _ = ring.complete(sqe.user_data, -(Error::NotSupported as i32));
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
        for &b in buf {
            self.putchar(b);
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
