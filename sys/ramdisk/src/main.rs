#![no_std]
#![no_main]

#[macro_use]
extern crate glenda;

extern crate alloc;
mod driver;
mod layout;
mod server;

use crate::layout::DEVICE_SLOT;
use driver::Ramdisk;
use glenda::cap::{
    CSPACE_CAP, CapPtr, CapType, ENDPOINT_CAP, ENDPOINT_SLOT, Endpoint, MONITOR_CAP, RECV_SLOT,
    REPLY_SLOT, Reply, VSPACE_CAP,
};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{DEVICE_ENDPOINT, ResourceType};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct RamdiskService<'a> {
    ramdisk: Option<Ramdisk>,
    endpoint: Endpoint,
    reply: Reply,
    recv: CapPtr,
    running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
    vspace_mgr: &'a mut VSpaceManager,
    cspace_mgr: &'a mut CSpaceManager,
    connected_client: Option<usize>,
}

impl<'a> RamdiskService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        vspace_mgr: &'a mut VSpaceManager,
        cspace_mgr: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            ramdisk: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
            vspace_mgr,
            cspace_mgr,
            connected_client: None,
        }
    }
}

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("Ramdisk");
    log!("Starting Ramdisk driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);
    let mut vspace_mgr = VSpaceManager::new(VSPACE_CAP.into(), 0x1000_0000, 0x1000_0000);
    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 16);

    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get device endpoint cap");
    let mut dev_client = DeviceClient::new(Endpoint::from(DEVICE_SLOT));

    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");

    let mut service =
        RamdiskService::new(&mut dev_client, &mut res_client, &mut vspace_mgr, &mut cspace_mgr);
    service.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    if let Err(e) = SystemService::init(&mut service) {
        error!("Failed to initialize: {:?}", e);
        return 1;
    }

    if let Err(e) = service.run() {
        error!("Exited with error: {:?}", e);
        return 1;
    }
    0
}
