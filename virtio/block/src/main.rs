#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

use glenda::cap::{CapPtr, Endpoint};
use glenda::interface::DriverService;
use glenda::interface::SystemService;
use glenda::protocol::device::DeviceNode;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("VirtIO-Blk: {}", format_args!($($arg)*));
    })
}

mod blk;
mod server;
pub use blk::VirtIOBlk;
pub use server::BlockService;

#[no_mangle]
fn main() -> usize {
    let mut service = BlockService::new();

    // Standard service layout
    service.listen(Endpoint::from(CapPtr::from(12)), CapPtr::from(1)).unwrap();

    // Initial discovery
    let node = DeviceNode {
        id: 1,
        compatible: alloc::string::String::from("virtio,mmio"),
        base_addr: 0x10001000,
        size: 0x1000,
        irq: 1,
        kind: glenda::utils::platform::DeviceKind::Virtio,
        parent_id: None,
        children: alloc::vec::Vec::new(),
    };
    DriverService::init(&mut service, node);

    service.run().expect("Block service crashed");
    0
}
