#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;
use glenda::cap::pagetable::{perms, Perms};
use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler, VSPACE_CAP};
use glenda::ipc::{MsgTag, UTCB};
use glenda::protocol::device as protocol;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("NS16550A: {}", format_args!($($arg)*));
    })
}
#[cfg(feature = "unicode")]
pub mod utf8;

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

struct Ns16550a {
    base: usize,
    irq: IrqHandler,
    #[cfg(feature = "unicode")]
    decoder: utf8::Utf8Decoder,
}

impl Ns16550a {
    fn new(base: usize, irq: IrqHandler) -> Self {
        Self {
            base,
            irq,
            #[cfg(feature = "unicode")]
            decoder: utf8::Utf8Decoder::new(),
        }
    }

    unsafe fn read(&self, offset: usize) -> u8 {
        let ptr = (self.base + offset) as *const u8;
        core::ptr::read_volatile(ptr)
    }

    unsafe fn write(&self, offset: usize, val: u8) {
        let ptr = (self.base + offset) as *mut u8;
        core::ptr::write_volatile(ptr, val);
    }

    fn init(&self) {
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

    fn handle_irq(&mut self) {
        unsafe {
            let iir = self.read(IIR);
            if iir & 0x01 == 0 {
                // Interrupt pending
                while let Some(b) = self.getchar() {
                    #[cfg(feature = "unicode")]
                    {
                        if b < 128 {
                            self.handle_char(b as char);
                        } else {
                            match self.decoder.push(b) {
                                utf8::Utf8PushResult::Completed(c) => self.handle_char(c),
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
            }
        }
    }

    fn handle_char(&mut self, c: char) {
        match c {
            '\r' => {
                self.put_char('\n');
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
                            unsafe {
                                self.putchar(0x08);
                            }
                        }
                        for _ in 0..w {
                            unsafe {
                                self.putchar(b' ');
                            }
                        }
                        for _ in 0..w {
                            unsafe {
                                self.putchar(0x08);
                            }
                        }
                    }
                }
                #[cfg(not(feature = "unicode"))]
                self.putchar(0x08);
                self.putchar(b' ');
                self.putchar(0x08);
            }
            _ => {
                self.put_char(c);
                #[cfg(feature = "unicode")]
                {
                    let w = utf8::char_display_width(c);
                    utf8::CONSOLE_ECHO.lock().push_width(w);
                }
            }
        }
    }

    fn put_char(&self, c: char) {
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

impl core::fmt::Write for Ns16550a {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            self.put_char(c);
        }
        Ok(())
    }
}

#[no_mangle]
fn main() -> usize {
    // Initialize logging (assuming cap 8 is console)
    log!("NS16550A Driver starting...");

    let unicorn = Endpoint::from(CapPtr::from(11)); // Unicorn endpoint shared by Unicorn service

    // 1. Find device by name
    let utcb = UTCB::current();
    utcb.clear();
    let (_offset, len) = utcb.append_str("uart@10000000").expect("Failed to append name");

    let tag = MsgTag::new(protocol::UNICORN_PROTO, 2);
    let args = [protocol::GET_DEVICE_BY_NAME, len, 0, 0, 0, 0, 0, 0];
    unicorn.call(tag, args);
    let device_id = UTCB::current().mrs_regs[0];

    if device_id == usize::MAX {
        log!("Device 'uart@10000000' not found");
        loop {}
    }
    log!("Found device ID {}", device_id);

    // 2. Map MMIO
    let mmio_slot = 20;
    let tag = MsgTag::new(protocol::UNICORN_PROTO, 4);
    let args = [protocol::MAP_MMIO, device_id, 0, mmio_slot, 0, 0, 0, 0];
    unicorn.call(tag, args);
    if UTCB::current().mrs_regs[0] != 0 {
        log!("Failed to map MMIO");
        loop {}
    }

    // Map into our VSpace
    let vspace = VSPACE_CAP;
    let mmio_va = 0x5000_0000;
    vspace.map(
        Frame::from(CapPtr::from(mmio_slot)),
        mmio_va,
        Perms::from(perms::READ | perms::WRITE | perms::USER),
    );
    log!("MMIO mapped at {:#x}", mmio_va);
    // 3. Get IRQ
    let irq_slot = 21;
    let tag = MsgTag::new(protocol::UNICORN_PROTO, 4);
    let args = [protocol::GET_IRQ, device_id, 0, irq_slot, 0, 0, 0, 0];
    unicorn.call(tag, args);
    if UTCB::current().mrs_regs[0] != 0 {
        log!("Failed to get IRQ");
        loop {}
    }
    log!("IRQ capability obtained at slot {}", irq_slot);

    let mut uart = Ns16550a::new(mmio_va, IrqHandler::from(CapPtr::from(irq_slot)));
    uart.init();

    use core::fmt::Write;
    let _ = write!(uart, "\nNS16550A Driver Started\n");
    let _ = write!(uart, "Unicode Test: Hello 🦀, 你好世界!\n");

    // 4. Bind IRQ to an endpoint for notifications
    let irq_ep_slot = 22;
    // Request a new endpoint from Factotum
    let factotum = Endpoint::from(CapPtr::from(10));
    let tag = MsgTag::new(glenda::protocol::process::FACTOTUM_PROTO, 5);
    factotum.call(tag, args);
    if UTCB::current().mrs_regs[0] != 0 {
        log!("Failed to allocate IRQ endpoint");
        loop {}
    }
    let irq_ep = Endpoint::from(CapPtr::from(irq_ep_slot));

    // Bind
    uart.irq.set_notification(irq_ep);
    uart.irq.ack();

    log!("Initialized and listening for interrupts...");

    loop {
        irq_ep.recv(0);
        uart.handle_irq();
        uart.irq.ack();
    }
    1
}
