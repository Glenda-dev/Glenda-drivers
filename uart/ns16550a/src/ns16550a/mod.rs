mod config;
mod consts;
#[cfg(feature = "unicode")]
mod utf8;

use config::*;
use consts::*;
use glenda::cap::{Frame, IrqHandler};
use glenda::error::Error;
use glenda::io::uring::IoUringServer;
use glenda::mem::shm::SharedMemory;
use glenda_drivers::interface::UartDriver;
#[cfg(feature = "unicode")]
use utf8::Utf8Decoder;

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    #[cfg(feature = "unicode")]
    decoder: Utf8Decoder,
    pub ring: Option<IoUringServer>,
    pub shm: Option<SharedMemory>,
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
        }
    }

    pub fn set_ring_server(&mut self, ring: IoUringServer) {
        self.ring = Some(ring);
    }

    pub fn setup_shm(
        &mut self,
        frame: Frame,
        vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> Result<(), Error> {
        let mut shm = SharedMemory::new(frame, vaddr, size);
        shm.set_paddr(paddr);
        self.shm = Some(shm);
        Ok(())
    }

    pub unsafe fn read(&self, offset: usize) -> u8 {
        let ptr = (self.base + offset) as *const u8;
        core::ptr::read_volatile(ptr)
    }

    pub unsafe fn write(&self, offset: usize, val: u8) {
        let ptr = (self.base + offset) as *mut u8;
        core::ptr::write_volatile(ptr, val);
    }

    pub fn init_hw(&self) {
        self.set_baud_rate(DEFAULT_BAUD_RATE);
        unsafe {
            // Enable RX interrupt
            self.write(IER, IER_RX_ENABLE);
        }
    }

    pub fn set_baud_rate(&self, baud: u32) {
        let divisor = calculate_divisor(baud);
        unsafe {
            // Disable interrupts during init
            self.write(IER, 0x00);

            // Enable DLAB to set baud rate
            self.write(LCR, LCR_DLAB);

            // Set divisor
            self.write(DLL, (divisor & 0xFF) as u8);
            self.write(DLM, (divisor >> 8) as u8);

            // 8 bits, no parity, one stop bit (8N1), disable DLAB
            self.write(LCR, LCR_DATA_BITS_8 | LCR_STOP_BITS_1 | LCR_PARITY_NONE);

            // Enable FIFO, clear them, with 14-byte threshold
            self.write(FCR, FCR_FIFO_ENABLE | FCR_FIFO_RX_RESET | FCR_FIFO_TX_RESET);

            // IRQs enabled, RTS/DTR set
            self.write(MCR, MCR_OUT2 | MCR_RTS | MCR_DTR);
        }
    }

    pub fn handle_irq(&mut self) -> Option<u8> {
        unsafe {
            let iir = self.read(IIR);
            if iir & IIR_NO_INTERRUPT == 0 {
                // Interrupt pending
                return self.getchar();
            }
        }
        None
    }

    pub fn handle_char(&mut self, b: u8) {
        #[cfg(feature = "unicode")]
        {
            if b < 128 {
                self.process_char(b as char);
            } else {
                match self.decoder.push(b) {
                    utf8::Utf8PushResult::Completed(c) => self.process_char(c),
                    utf8::Utf8PushResult::Invalid => {}
                    utf8::Utf8PushResult::Pending => {}
                }
            }
        }
        #[cfg(not(feature = "unicode"))]
        {
            // Echo back
            if b == b'\r' {
                self.putchar(b'\n');
            } else {
                self.putchar(b);
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
                        let buf = if let Some(shm) = &self.shm {
                            // Convert client-side addr to server-side vaddr
                            let server_vaddr = shm.vaddr() + (addr as usize - shm.client_vaddr());
                            unsafe { core::slice::from_raw_parts(server_vaddr as *const u8, len) }
                        } else {
                            unsafe { core::slice::from_raw_parts(addr as *const u8, len) }
                        };

                        for &b in buf {
                            self.putchar(b);
                        }
                        let _ = ring.complete(user_data, len as i32);
                    }
                    glenda::io::uring::IOURING_OP_READ => {
                        // For UART, READ SQEs could be queued and completed when data arrives.
                        // For now, let's just do a synchronous check.
                        let addr = sqe.addr;
                        let len = sqe.len as usize;
                        let user_data = sqe.user_data;

                        let mut count = 0;
                        while count < len {
                            if let Some(c) = self.getchar() {
                                if let Some(shm) = &self.shm {
                                    let server_vaddr =
                                        shm.vaddr() + (addr as usize - shm.client_vaddr()) + count;
                                    unsafe { *(server_vaddr as *mut u8) = c };
                                } else {
                                    unsafe { *((addr as usize + count) as *mut u8) = c };
                                }
                                count += 1;
                            } else {
                                break;
                            }
                        }

                        if count > 0 {
                            let _ = ring.complete(user_data, count as i32);
                        } else {
                            // If no data, we should probably re-queue or return 0.
                            // To be truly async, we'd store the SQE and complete it in handle_irq.
                            let _ = ring.complete(user_data, 0);
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

    fn process_char(&mut self, c: char) {
        match c {
            '\r' => {
                self.putchar('\n' as u8);
                #[cfg(feature = "unicode")]
                utf8::CONSOLE_ECHO.lock().clear_line();
            }
            '\x08' | '\x7f' => {
                // Backspace
                #[cfg(feature = "unicode")]
                {
                    let mut echo = utf8::CONSOLE_ECHO.lock();
                    if let Some(w) = echo.pop_width() {
                        for _ in 0..w {
                            self.putchar(0x08);
                        }
                        for _ in 0..w {
                            self.putchar(b' ');
                        }
                        for _ in 0..w {
                            self.putchar(0x08);
                        }
                    }
                }
                #[cfg(not(feature = "unicode"))]
                {
                    self.putchar(0x08);
                    self.putchar(b' ');
                    self.putchar(0x08);
                }
            }
            _ => {
                self.put_unicode_char(c);
                #[cfg(feature = "unicode")]
                {
                    let w = utf8::char_display_width(c);
                    utf8::CONSOLE_ECHO.lock().push_width(w);
                }
            }
        }
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
            if self.read(LSR) & LSR_DATA_READY != 0 {
                Some(self.read(RBR))
            } else {
                None
            }
        }
    }

    fn putchar(&self, c: u8) {
        unsafe {
            while self.read(LSR) & LSR_THR_EMPTY == 0 {}
            self.write(THR, c);
        }
    }
}

impl UartDriver for Ns16550a {
    fn put_char(&mut self, c: u8) {
        self.putchar(c);
    }

    fn get_char(&mut self) -> Option<u8> {
        self.getchar()
    }

    fn put_str(&mut self, s: &str) {
        for c in s.bytes() {
            self.putchar(c);
        }
    }

    fn set_baud_rate(&mut self, baud: u32) {
        Self::set_baud_rate(self, baud);
    }
}

impl core::fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.put_str(s);
        Ok(())
    }
}
