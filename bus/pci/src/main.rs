#![no_std]
#![no_main]
#![allow(dead_code)]

#[macro_use]
extern crate glenda;

extern crate alloc;
use glenda::cap::MONITOR_CAP;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{DeviceService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::init::ServiceState;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

mod driver;
mod layout;
mod pci;
mod platform;

use crate::driver::PciBusDriver;
use crate::layout::{DEVICE_CAP, DEVICE_SLOT, ENDPOINT_SLOT};

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("PCI");
    log!("Starting PCI Bus Driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // Get Device Manager endpoint
    if let Err(e) =
        res_client.get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
    {
        log!("Failed to get device endpoint: {:?}", e);
        return 1;
    }

    let mut dev_client = DeviceClient::new(DEVICE_CAP);
    let mut cspace_mgr = CSpaceManager::new(glenda::cap::CSPACE_CAP, 16);
    let mut vspace_mgr =
        VSpaceManager::new(glenda::cap::VSPACE_CAP.into(), 0x6000_0000, 0x1000_0000);

    let status = {
        let mut driver = PciBusDriver::new(
            glenda::cap::Endpoint::from(ENDPOINT_SLOT),
            &mut dev_client,
            &mut res_client,
            &mut vspace_mgr,
            &mut cspace_mgr,
        );

        log!("Scanning for PCI Host Bridge...");

        match driver.scan() {
            Ok(_) => ServiceState::Running,
            Err(e) => {
                log!("PCI Scan failed: {:?}", e);
                ServiceState::Failed
            }
        }
    };

    if let Err(e) = dev_client.report_state(Badge::null(), status) {
        warn!("Failed to report driver state: {:?}", e);
    }
    if status == ServiceState::Failed {
        return 1;
    }

    log!("PCI Enumeration finished.");
    0
}
