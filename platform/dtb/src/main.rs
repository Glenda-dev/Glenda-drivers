#![no_std]
#![no_main]

#[macro_use]
extern crate glenda;

extern crate alloc;
mod driver;
mod layout;
mod server;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT, ENDPOINT_SLOT};
pub use driver::DtbDriver;

use glenda::cap::{CapType, Endpoint, MONITOR_CAP};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::ResourceType;

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("DTB");
    log!("Starting DTB Platform Driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // 1. Get Device Manager (Unicorn) endpoint
    if let Err(e) = res_client.get_cap(
        Badge::null(),
        ResourceType::Endpoint,
        glenda::protocol::resource::DEVICE_ENDPOINT,
        DEVICE_SLOT,
    ) {
        error!("Failed to get device endpoint: {:?}", e);
        return 1;
    }
    if let Err(e) = res_client.alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT) {
        error!("Failed to allocate endpoint slot: {:?}", e);
        return 1;
    }

    let dev_client = DeviceClient::new(DEVICE_CAP);
    let mut driver = DtbDriver::new(Endpoint::from(ENDPOINT_SLOT), dev_client, res_client);

    // 2. Interact with SystemService for initialization
    if let Err(e) = SystemService::init(&mut driver) {
        error!("Init failed: {:?}", e);
        return 1;
    }

    // 3. Start Server Loop
    if let Err(e) = SystemService::run(&mut driver) {
        error!("Run failed: {:?}", e);
        return 1;
    }

    0
}
