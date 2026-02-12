#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

mod driver;
mod gpio;
mod layout;
mod server;

pub use gpio::SiFiveGpio;
pub use server::GpioService;

use crate::layout::{DEVICE_CAP, DEVICE_SLOT};
use glenda::cap::CapType;
use glenda::cap::{ENDPOINT_CAP, ENDPOINT_SLOT, MONITOR_CAP, RECV_SLOT, REPLY_SLOT};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{ResourceType, DEVICE_ENDPOINT};

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}SiFive-GPIO: {}{}", glenda::console::ANSI_BLUE, format_args!($($arg)*), glenda::console::ANSI_RESET);
    })
}

#[no_mangle]
fn main() -> usize {
    log!("Starting...");
    let mut res_client = ResourceClient::new(MONITOR_CAP);
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get device endpoint cap");
    let mut dev_client = DeviceClient::new(DEVICE_CAP);

    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");

    let mut service = GpioService::new(&mut dev_client, &mut res_client);
    service.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    SystemService::init(&mut service).expect("Failed to init GPIO service");

    service.run().expect("GPIO service crashed");
    0
}
