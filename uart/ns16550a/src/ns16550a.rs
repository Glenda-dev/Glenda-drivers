#[cfg(feature = "unicode")]
use crate::utf8;
#[cfg(feature = "unicode")]
use crate::Utf8Decoder;
use glenda::cap::IrqHandler;
use glenda::interface::device::UartDevice;

// Registers
const RBR: usize = 0;
const THR: usize = 0;
const IER: usize = 1;
const IIR: usize = 2;
const FCR: usize = 2;
const LCR: usize = 3;
const MCR: usize = 4;
const LSR: usize = 5;
const MSR: usize = 6;
const SCR: usize = 7;

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    #[cfg(feature = "unicode")]
    decoder: Utf8Decoder,
}

impl Ns16550a {
    pub fn new(base: usize, irq: IrqHandler) -> Self {
        Self {
            base,
            irq,
            #[cfg(feature = "unicode")]
            decoder: Utf8Decoder::new(),
        }
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
        unsafe {
            self.write(IER, 0x00); // Disable interrupts
            self.write(LCR, 0x80); // Enable DLAB
            self.write(0, 0x03); // Divisor low (38.4K)
            self.write(1, 0x00); // Divisor high
            self.write(LCR, 0x03); // 8 bits, no parity, one stop bit
            self.write(FCR, 0x07); // Enable FIFO, clear them
            self.write(MCR, 0x0B); // IRQs enabled, RTS/DSR set
            self.write(IER, 0x01); // Enable RX interrupt
        }
    }

    pub fn handle_irq(&mut self) -> Option<u8> {
        unsafe {
            let iir = self.read(IIR);
            if iir & 0x01 == 0 {
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
            if self.read(LSR) & 0x01 != 0 {
                Some(self.read(RBR))
            } else {
                None
            }
        }
    }

    fn putchar(&self, c: u8) {
        unsafe {
            while self.read(LSR) & 0x20 == 0 {}
            self.write(THR, c);
        }
    }
}

impl UartDevice for Ns16550a {
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
}

impl core::fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.put_str(s);
        Ok(())
    }
}
