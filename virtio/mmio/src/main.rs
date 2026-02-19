#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}VirtIO-MMIO: {}{}", glenda::console::ANSI_BLUE, format_args!($($arg)*), glenda::console::ANSI_RESET);
    })
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => ({
        glenda::println!("{}VirtIO-MMIO: {}{}", glenda::console::ANSI_RED, format_args!($($arg)*), glenda::console::ANSI_RESET);
    })
}

mod driver;
mod layout;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use glenda::cap::MONITOR_CAP;
use glenda::client::{DeviceClient, ResourceClient};
use glenda_drivers::interface::ProbeDriver;
use glenda::interface::{DeviceService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};

#[unsafe(no_mangle)]
fn main() -> usize {
    log!("Driver starting...");
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // Get Device Manager endpoint
    if let Err(e) =
        res_client.get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
    {
        error!("Failed to get device endpoint: {:?}", e);
        return 1;
    }

    let mut dev_client = DeviceClient::new(DEVICE_CAP);
    let mut driver = driver::VirtioMmioDriver::new(&mut dev_client, &mut res_client);

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
    0
}
