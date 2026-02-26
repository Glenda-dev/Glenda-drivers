mod device;
mod driver;
mod server;

use crate::Ns16550a;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::utils::manager::CSpaceManager;

pub struct UartService<'a> {
    pub(crate) uart: Option<Ns16550a>,
    pub(crate) endpoint: Endpoint,
    pub(crate) reply: Reply,
    pub(crate) recv: CapPtr,
    pub(crate) irq_ep: Endpoint,
    pub(crate) running: bool,

    pub(crate) dev: &'a mut DeviceClient,
    pub(crate) res: &'a mut ResourceClient,
    pub(crate) cspace: &'a mut CSpaceManager,
}

impl<'a> UartService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            uart: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            irq_ep: Endpoint::from(CapPtr::null()),
            dev: dev,
            res: res,
            cspace: cspace,
            running: false,
        }
    }
}
