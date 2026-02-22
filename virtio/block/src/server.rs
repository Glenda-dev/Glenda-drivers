use crate::blk::*;
use crate::layout::RING_VA;
use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{MemoryService, ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, UTCB};
use glenda::mem::shm::SharedMemory;
use glenda::utils::manager::{CSpaceManager, CSpaceService};
use glenda_drivers::interface::BlockDriver;
use glenda_drivers::interface::DriverService;
use glenda_drivers::io_uring::{IoRing, IoRingServer};
use glenda_drivers::protocol::{block, BLOCK_PROTO};

pub struct BlockService<'a> {
    pub blk: Option<VirtIOBlk>,
    pub irq: Option<IrqHandler>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub recv: CapPtr,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
}

pub const IRQ_BADGE: Badge = Badge::new(0x80);

impl<'a> BlockService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            blk: None,
            irq: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply_cap: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
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

    fn setup_ring(
        &mut self,
        sq_entries: u32,
        cq_entries: u32,
        notify_ep: Endpoint,
        _recv: CapPtr,
    ) -> Result<Frame, Error> {
        let slot = self.cspace_mgr.alloc(self.res)?;
        // For 4 entries, we only need a few hundred bytes, so 1 page is plenty.
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;
        log!("setup_ring: dma_alloc ok, paddr={:#x}", paddr);

        // Map the DMA frame to our virtual address space
        self.res.mmap(Badge::null(), frame.clone(), RING_VA, glenda::arch::mem::PGSIZE)?;
        glenda::arch::sync::fence();

        let shm = SharedMemory::from_frame(frame.clone(), RING_VA, glenda::arch::mem::PGSIZE);

        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        let mut server = IoRingServer::new(ring);

        server.set_client_notify(notify_ep);
        server.set_notify_tag(glenda::ipc::MsgTag::new(
            BLOCK_PROTO,
            block::NOTIFY_IO,
            glenda::ipc::MsgFlags::NONE,
        ));

        if let Some(blk) = self.blk.as_mut() {
            blk.set_ring_server(server);
        } else {
            error!("setup_ring: blk is None!");
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
        if let Some(blk) = self.blk.as_mut() {
            blk.setup_shm(frame, vaddr, paddr, size)
        } else {
            Err(Error::NotInitialized)
        }
    }
}

impl<'a> SystemService for BlockService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, endpoint: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = endpoint;
        self.reply_cap = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        let mut utcb = unsafe { UTCB::new() };
        loop {
            utcb.clear();
            utcb.set_reply_window(self.reply_cap.cap());
            utcb.set_recv_window(self.recv);

            if let Err(e) = self.endpoint.recv(&mut utcb) {
                glenda::println!("BlockService recv error: {:?}", e);
                continue;
            }
            match self.dispatch(&mut utcb) {
                Ok(_) => {
                    let _ = self.reply(&mut utcb);
                }
                Err(Error::Success) => {
                    // Successfully handled, but no reply needed (e.g., notification)
                }
                Err(e) => {
                    glenda::println!("BlockService dispatch error: {:?}", e);
                    utcb.set_msg_tag(glenda::ipc::MsgTag::err());
                    utcb.set_mr(0, e as usize);
                    let _ = self.reply(&mut utcb);
                }
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge();

        if badge & IRQ_BADGE != Badge::null() {
            return handle_notify(utcb, |_| {
                if let Some(blk) = self.blk.as_mut() {
                    blk.handle_irq();
                    if let Some(irq) = self.irq.as_ref() {
                        irq.ack()?;
                    }
                }
                Ok(())
            });
        }

        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(blk) = s.blk.as_mut() {
                        blk.handle_irq();
                        if let Some(irq) = s.irq.as_ref() {
                            irq.ack()?;
                        }
                    }
                    Ok(())
                })
            },
            (BLOCK_PROTO, block::NOTIFY_SQ) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(blk) = s.blk.as_mut() {
                        blk.handle_ring();
                    }
                    Ok(())
                })
            },
            (BLOCK_PROTO, block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.capacity() as usize))
            },
            (BLOCK_PROTO, block::GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.block_size() as usize))
            },
            (BLOCK_PROTO, block::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let vaddr = u.get_mr(0);
                    let size = u.get_mr(1);
                    let paddr = u.get_mr(2) as u64;
                    // Move the cap from recv window to a temporary slot
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    s.cspace_mgr.root().move_cap(s.recv, slot)?;
                    let frame = Frame::from(slot);
                    s.setup_shm(frame, vaddr, paddr, size)?;
                    Ok(0usize)
                })
            },
            (BLOCK_PROTO, block::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                 handle_cap_call(u, |u| {
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;

                    // The client passed its notification endpoint in the UTCB.
                    // It was placed in our recv_window (s.recv).
                    // We need to move it to a permanent slot.
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    s.cspace_mgr.root().move_cap(s.recv, slot)?;
                    let notify_ep = Endpoint::from(slot);

                    let frame = s.setup_ring(sq, cq, notify_ep, CapPtr::null())?;
                    Ok(frame.cap())
                })
            },
            (_, _) => |s: &mut Self, u: &mut UTCB| {
                let tag = u.get_msg_tag();
                if tag.proto() == 0 && tag.label() == 0 {
                     return handle_notify(u, |_u| {
                        if let Some(blk) = s.blk.as_mut() {
                            blk.handle_irq();
                        }
                        Ok(())
                    });
                }
                Err(Error::NotSupported)
            }
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply_cap.reply(utcb)
    }

    fn stop(&mut self) {}
}
