use crate::blk::*;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call};
use glenda::ipc::{Badge, UTCB};
use glenda::mem::shm::SharedMemory;
use glenda::utils::manager::{CSpaceManager, CSpaceService};
use glenda_drivers::interface::BlockDriver;
use glenda_drivers::interface::DriverService;
use glenda_drivers::io_uring::{IoRing, IoRingServer};
use glenda_drivers::protocol::{block, BLOCK_PROTO};

pub struct BlockService<'a> {
    pub blk: Option<VirtIOBlk>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
}

pub const IRQ_BADGE: Badge = Badge::new(0x1);

impl<'a> BlockService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            blk: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply_cap: Reply::from(CapPtr::null()),
            dev,
            res,
            cspace_mgr,
        }
    }
}

impl<'a> BlockDriver for BlockService<'a> {
    fn capacity(&self) -> u64 {
        self.blk.as_ref().map(|b| b.capacity()).unwrap_or(0)
    }

    fn block_size(&self) -> u32 {
        self.blk.as_ref().map(|b| b.block_size()).unwrap_or(0)
    }

    fn setup_ring(&mut self, sq_entries: u32, cq_entries: u32) -> Result<Frame, Error> {
        let slot = self.cspace_mgr.alloc(self.res)?;
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, slot)?;

        let shm = SharedMemory::from_frame(frame.clone(), paddr as usize, 16384);
        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        let mut server = IoRingServer::new(ring);

        server.set_client_notify(self.endpoint.clone());

        if let Some(blk) = self.blk.as_mut() {
            blk.set_ring_server(server);
        }

        Ok(frame)
    }
}

impl<'a> SystemService for BlockService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, endpoint: Endpoint, reply: CapPtr, _recv: CapPtr) -> Result<(), Error> {
        self.endpoint = endpoint;
        self.reply_cap = Reply::from(reply);
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        let mut utcb = unsafe { UTCB::new() };
        loop {
            if let Err(e) = self.endpoint.recv(&mut utcb) {
                glenda::println!("BlockService recv error: {:?}", e);
                continue;
            }
            if let Err(e) = self.dispatch(&mut utcb) {
                glenda::println!("BlockService dispatch error: {:?}", e);
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge();

        if badge & IRQ_BADGE != Badge::null() {
            if let Some(blk) = self.blk.as_mut() {
                blk.handle_irq();
            }
            return Ok(());
        }

        let tag = utcb.get_msg_tag();
        if tag.proto() == 0 {
            if let Some(blk) = self.blk.as_mut() {
                blk.handle_ring();
            }
            return Ok(());
        }

        glenda::ipc_dispatch! {
            self, utcb,
            (BLOCK_PROTO, block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.capacity() as usize))
            },
            (BLOCK_PROTO, block::GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.block_size() as usize))
            },
            (BLOCK_PROTO, block::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                 handle_cap_call(u, |u| {
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;
                    let frame = s.setup_ring(sq, cq)?;
                    Ok(frame.cap())
                })
            },
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply_cap.reply(utcb)
    }

    fn stop(&mut self) {}
}
