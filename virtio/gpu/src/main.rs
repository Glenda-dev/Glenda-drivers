#![no_std]
#![no_main]
#![allow(dead_code)]

#[macro_use]
extern crate glenda;

extern crate alloc;

mod gpu;
mod layout;
mod protocol;
mod server;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use crate::server::GpuService;
use glenda::cap::{CapType, CSPACE_CAP, ENDPOINT_CAP, MONITOR_CAP, REPLY_SLOT, VSPACE_CAP};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{DeviceService, ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::ResourceType;
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

#[no_mangle]
fn main() -> usize {
    glenda::console::init_logging("VirtIO-GPU");
    let mut res_client = ResourceClient::new(MONITOR_CAP);
    res_client
        .get_cap(
            Badge::null(),
            ResourceType::Endpoint,
            glenda::protocol::resource::DEVICE_ENDPOINT,
            DEVICE_SLOT,
        )
        .unwrap();
    let mut dev_client = DeviceClient::new(DEVICE_CAP);
    res_client.alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_CAP.cap()).unwrap();

    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 32);
    let mut vspace_mgr = VSpaceManager::new(VSPACE_CAP, 0x8000_0000, 0x9000_0000);

    let mut service = GpuService::new(
        &mut dev_client,
        &mut res_client,
        &mut cspace_mgr,
        &mut vspace_mgr,
        ENDPOINT_CAP,
        REPLY_SLOT,
    );
    if let Err(e) = service.init() {
        error!("Failed to initialize GPU service: {:?}", e);
        let _ =
            service.dev.report_state(Badge::null(), glenda::protocol::init::ServiceState::Failed);
        return 1;
    }
    if let Err(e) =
        service.dev.report_state(Badge::null(), glenda::protocol::init::ServiceState::Running)
    {
        warn!("Failed to report driver running state: {:?}", e);
    }
    if let Err(e) = service.run() {
        error!("Failed to run GPU service: {:?}", e);
        return 1;
    }
    0
}
