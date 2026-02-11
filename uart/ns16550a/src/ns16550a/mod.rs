mod config;
mod consts;
#[cfg(feature = "unicode")]
mod utf8;

use config::*;
use consts::*;
use glenda::cap::IrqHandler;
use glenda::interface::drivers::UartDriver;
#[cfg(feature = "unicode")]
use utf8::Utf8Decoder;

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
        let divisor = calculate_divisor(DEFAULT_BAUD_RATE);
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

            // Enable RX interrupt
            self.write(IER, IER_RX_ENABLE);
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
}

impl core::fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.put_str(s);
        Ok(())
    }
}
