#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}VirtIO-Blk: {}{}", glenda::console::ANSI_BLUE,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => ({
        glenda::println!("{}VirtIO-Blk: {}{}", glenda::console::ANSI_RED,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}

mod blk;
mod driver;
mod layout;
mod server;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use glenda::cap::CapType;
use glenda::cap::{CSPACE_CAP, ENDPOINT_CAP, ENDPOINT_SLOT, MONITOR_CAP, RECV_SLOT, REPLY_SLOT};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};
use glenda::utils::manager::CSpaceManager;

pub use blk::VirtIOBlk;
pub use server::BlockService;

#[no_mangle]
fn main() -> usize {
    log!("VirtIO-Blk Driver starting...");
    let mut res_client = ResourceClient::new(MONITOR_CAP);
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get device endpoint cap");
    let mut dev_client = DeviceClient::new(DEVICE_CAP);

    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");

    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 16);
    let mut service = BlockService::new(&mut dev_client, &mut res_client, &mut cspace_mgr);
    service.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    SystemService::init(&mut service).expect("Failed to init block service");

    service.run().expect("Block service crashed");
    0
}
