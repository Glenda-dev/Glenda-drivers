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
        glenda::println!("VirtIO-Net: {}", format_args!($($arg)*));
    })
}

mod net;
mod server;
pub use server::NetService;

#[no_mangle]
fn main() -> usize {
    let mut service = NetService::new();

    // Standard service layout (similar to BLK)
    // We assume we are started by Warren and given an endpoint to listen on.
    // For now we hardcode similar to BLK example, or assume passed handles.
    // BLK example: service.listen(Endpoint::from(CapPtr::from(12)), CapPtr::from(1)).unwrap();

    // Check if we can just rely on standard init if SystemService trait covers it?
    // SystemService has `listen`.
    // We manually call listen for now.
    service.listen(Endpoint::from(CapPtr::from(12)), glenda::cap::REPLY_SLOT).unwrap();

    // Initial discovery (Mocking the node info passed by system manager)
    let node = DeviceNode {
        id: 2, // ID 2 for Net? BLK was 1 in example.
        compatible: alloc::string::String::from("virtio,mmio"),
        base_addr: 0x10002000, // Different from BLK
        size: 0x1000,
        irq: 2,
        kind: glenda::utils::platform::DeviceKind::Virtio,
        parent_id: None,
        children: alloc::vec::Vec::new(),
    };
    DriverService::init(&mut service, node);

    service.run().expect("Net service crashed");
    0
}
