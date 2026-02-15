#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;
use crate::layout::{DEVICE_CAP, DEVICE_SLOT, ENDPOINT_SLOT, MMIO_SLOT};
use glenda::cap::Endpoint;
use glenda::cap::MONITOR_CAP;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DeviceService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::resource::ResourceType;
use glenda::protocol::resource::DEVICE_ENDPOINT;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}ACPI: {}{}", glenda::console::ANSI_BLUE,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}

macro_rules! error {
    ($($arg:tt)*) => ({
        glenda::println!("{}ACPI: {}{}", glenda::console::ANSI_RED,format_args!($($arg)*),glenda::console::ANSI_RESET);
    })
}

mod arch;
mod driver;
mod handler;
mod layout;
mod probe;

pub use driver::AcpiDriver;

#[unsafe(no_mangle)]
fn main() -> usize {
    log!("Starting ACPI Platform Driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);

    if let Err(e) =
        res_client.get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
    {
        error!("Failed to get device endpoint: {:?}", e);
        return 1;
    }

    if let Err(e) = res_client.get_cap(Badge::null(), ResourceType::Mmio, 0, MMIO_SLOT) {
        error!("Failed to get MMIO cap: {:?}", e);
        return 1;
    }

    let mut dev_client = DeviceClient::new(DEVICE_CAP);

    let mut driver =
        AcpiDriver::new(Endpoint::from(ENDPOINT_SLOT), &mut dev_client, &mut res_client);

    log!("Probing...");
    match driver.probe() {
        Ok(devices) => {
            log!("Found {} devices", devices.len());
            // Report all at once
            if let Err(e) = dev_client.report(Badge::null(), devices) {
                error!("Failed to report devices: {:?}", e);
            }
        }
        Err(e) => {
            error!("Probe failed (not ACPI platform?): {:?}", e);
        }
    }

    log!("ACPI Driver finished.");
    loop {
        // Yield cpu? For now just loop.
    }
}
