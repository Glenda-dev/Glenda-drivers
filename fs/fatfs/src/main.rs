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

pub use server::FatFsService;

#[no_mangle]
fn main() -> usize {
    // In a real scenario, we would get the block device capability from the root task or device manager.
    // For now, we assume it's passed or well-known.
    let mut service = FatFsService::new();

    // Standard service setup would go here
    // service.listen(...);

    // service.run().expect("FatFs service crashed");
    0
}
