#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;
use crate::layout::{DEVICE_CAP, DEVICE_SLOT, ENDPOINT_SLOT};
use glenda::cap::Endpoint;
use glenda::cap::MONITOR_CAP;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DeviceService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}DTB: {}{}", glenda::console::ANSI_BLUE,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => ({
        glenda::println!("{}DTB: {}{}", glenda::console::ANSI_YELLOW,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => ({
        glenda::println!("{}DTB: {}{}", glenda::console::ANSI_RED,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}

mod driver;
mod layout;

pub use driver::DtbDriver;

#[unsafe(no_mangle)]
fn main() -> usize {
    log!("Starting DTB Platform Driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // Get Device Manager endpoint
    if let Err(e) =
        res_client.get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
    {
        error!("Failed to get device endpoint: {:?}", e);
        return 1;
    }

    let mut dev_client = DeviceClient::new(DEVICE_CAP);

    let mut driver =
        DtbDriver::new(Endpoint::from(ENDPOINT_SLOT), &mut dev_client, &mut res_client);

    log!("Probing...");
    match driver.probe() {
        Ok(devices) => {
            log!("Found {} devices", devices.len());

            // Log some info about root
            if let Some(root) = devices.first() {
                log!("Root device: {}", root.desc.name);
            }

            if let Err(e) = dev_client.report(Badge::null(), devices) {
                error!("Failed to report devices: {:?}", e);
            }
        }
        Err(e) => {
            error!("Probe failed (not DTB platform?): {:?}", e);
        }
    }

    log!("DTB Driver finished.");
    0
}
