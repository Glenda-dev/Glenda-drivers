#![no_std]
#![no_main]
#![allow(dead_code)]

#[macro_use]
extern crate glenda;

extern crate alloc;

mod driver;
mod layout;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use glenda::cap::MONITOR_CAP;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::drivers::interface::ProbeDriver;
use glenda::interface::{DeviceService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("VirtIO-MMIO");
    log!("Driver starting...");
    let mut res_client = ResourceClient::new(MONITOR_CAP);
    let mut cspace_mgr = CSpaceManager::new(glenda::cap::CSPACE_CAP, 16);
    let mut vspace_mgr = VSpaceManager::new(glenda::cap::VSPACE_CAP.into(), 0x1000_000, 0x1000_000);

    // Get Device Manager endpoint
    if let Err(e) =
        res_client.get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
    {
        error!("Failed to get device endpoint: {:?}", e);
        return 1;
    }

    let mut dev_client = DeviceClient::new(DEVICE_CAP);
    let mut driver = driver::VirtioMmioDriver::new(
        &mut dev_client,
        &mut res_client,
        &mut vspace_mgr,
        &mut cspace_mgr,
    );

    match driver.probe() {
        Ok(res) => {
            if !res.is_empty() {
                log!("Probe successful: {:?}", res);
                if let Err(e) = dev_client.update(Badge::null(), res) {
                    error!("Failed to update device info: {:?}", e);
                }
            } else {
                log!("Probe complete: no specific device identified.");
            }
        }
        Err(e) => {
            error!("Probe failed: {:?}", e);
        }
    }
    // FIXME
    let _ = glenda::cap::TCB_CAP.suspend();
    0
}
