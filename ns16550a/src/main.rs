#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;
use glenda::cap::{CapPtr, Endpoint};
use glenda::interface::device::DriverService;
use glenda::interface::system::SystemService;
use glenda::manager::device::DeviceNode;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("NS16550A: {}", format_args!($($arg)*));
    })
}
mod ns16550a;
mod server;
#[cfg(feature = "unicode")]
mod utf8;

pub use ns16550a::Ns16550a;
pub use server::UartService;
#[cfg(feature = "unicode")]
pub use utf8::Utf8Decoder;

#[no_mangle]
fn main() -> usize {
    log!("NS16550A Driver starting...");

    let mut service = UartService::new();

    // Setup initial caps (standard for services)
    service.listen(Endpoint::from(CapPtr::from(12)), CapPtr::from(1)).unwrap();

    // Discovery (Self-init for now as we don't have a manager calling us yet)
    // In a real system, Unicorn would call init() via IPC.
    let node = DeviceNode {
        id: 0,
        compatible: alloc::string::String::from("ns16550a"),
        base_addr: 0x10000000,
        size: 0x1000,
        irq: 10,
        kind: glenda::utils::platform::DeviceKind::Uart,
        parent_id: None,
        children: alloc::vec::Vec::new(),
    };
    DriverService::init(&mut service, node);

    service.run().unwrap();
    0
}
