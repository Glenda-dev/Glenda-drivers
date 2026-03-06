#![no_std]
#![no_main]
#![allow(dead_code)]

#[macro_use]
extern crate glenda;

extern crate alloc;

mod driver;
mod layout;
mod sdhci;
mod server;

use glenda::utils::manager::{CSpaceManager, VSpaceManager};
pub use server::SdhciService;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use glenda::cap::{CapType, CSPACE_CAP, VSPACE_CAP};
use glenda::cap::{ENDPOINT_CAP, ENDPOINT_SLOT, MONITOR_CAP, RECV_SLOT, REPLY_SLOT};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};

#[no_mangle]
fn main() -> usize {
    glenda::console::init_logging("SDHCI");
    log!("Starting...");
    let mut res_client = ResourceClient::new(MONITOR_CAP);
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get device endpoint cap");
    let mut dev_client = DeviceClient::new(DEVICE_CAP);

    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");
    let mut vspace_mgr = VSpaceManager::new(VSPACE_CAP, 0, 0);
    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 16);

    let mut service =
        SdhciService::new(&mut dev_client, &mut res_client, &mut vspace_mgr, &mut cspace_mgr);
    service.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    SystemService::init(&mut service).expect("Failed to init SDHCI service");

    service.run().expect("SDHCI service crashed");
    0
}
