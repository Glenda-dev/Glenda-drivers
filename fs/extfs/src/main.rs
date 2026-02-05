#![no_std]
#![no_main]

extern crate alloc;

use glenda::cap::{CapPtr, Endpoint};
use glenda::interface::system::SystemService;

mod block;
mod defs;
mod fs;
mod ops;
mod server;
mod versions;

pub use server::Ext4Service;

#[no_mangle]
fn main() -> usize {
    let mut _service = Ext4Service::new();
    // service.run().expect("Ext4 service crashed");
    0
}
