mod device;
mod driver;
mod server;

use crate::layout::{RING_VA, SHM_VA};
use crate::Ns16550a;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{CSpaceService, ResourceService, VSpaceService};
use glenda::io::uring::{IoUringBuffer, IoUringServer};
use glenda::ipc::Badge;
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct UartService<'a> {
    pub uart: Option<Ns16550a>,
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub irq_ep: Endpoint,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace: &'a mut CSpaceManager,
    pub vspace: &'a mut VSpaceManager,
    pub connected_client: Option<usize>,
}

impl<'a> UartService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace: &'a mut CSpaceManager,
        vspace: &'a mut VSpaceManager,
    ) -> Self {
        Self {
            uart: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            irq_ep: Endpoint::from(CapPtr::null()),
            dev,
            res,
            cspace,
            vspace,
            running: false,
            connected_client: None,
        }
    }
    fn setup_ring(&mut self, sq: u32, cq: u32, notify_ep: Endpoint) -> Result<Frame, Error> {
        log!("Setting up ring: sq={}, cq={}, notify_ep={}", sq, cq, notify_ep.cap());
        let slot = self.cspace.alloc(self.res)?;
        let (_paddr, frame): (usize, Frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;

        self.vspace.map_frame(
            frame.clone(),
            RING_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace,
        )?;

        let ring = unsafe { IoUringBuffer::new(RING_VA as *mut u8, 4096, sq, cq) };
        let mut server = IoUringServer::new(ring);
        server.set_client_notify(notify_ep);

        if let Some(uart) = self.uart.as_mut() {
            uart.ring = Some(server);
        }

        Ok(frame)
    }

    fn setup_shm(
        &mut self,
        frame: Frame,
        vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> Result<(), Error> {
        self.vspace.map_frame(
            frame.clone(),
            SHM_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace,
        )?;

        if let Some(uart) = self.uart.as_mut() {
            uart.setup_shm(frame, vaddr, paddr, size)?;
        }
        Ok(())
    }
}
