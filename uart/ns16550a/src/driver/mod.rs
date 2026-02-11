mod device;
mod driver;
mod server;

use crate::Ns16550a;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::{DeviceClient, ResourceClient};

pub struct UartService<'a> {
    uart: Option<Ns16550a>,
    endpoint: Endpoint,
    reply: Reply,
    recv: CapPtr,
    irq_ep: Endpoint,
    running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
}

impl<'a> UartService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            uart: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            irq_ep: Endpoint::from(CapPtr::null()),
            dev: dev,
            res: res,
            running: false,
        }
    }
}
