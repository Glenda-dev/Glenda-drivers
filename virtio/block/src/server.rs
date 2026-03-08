use crate::blk::*;
use crate::layout::{IRQ_BADGE, RING_VA};
use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler, Reply, CSPACE_CAP};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::drivers::interface::DriverService;
use glenda::drivers::protocol::{block, BLOCK_PROTO};
use glenda::error::Error;
use glenda::interface::{CSpaceService, ResourceService, SystemService, VSpaceService};
use glenda::io::uring::{IoUringBuffer as IoUring, IoUringServer};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct BlockService<'a> {
    pub blk: Option<VirtIOBlk>,
    pub irq: Option<IrqHandler>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub recv: CapPtr,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
    pub vspace_mgr: &'a mut VSpaceManager,
    pub connected_client: Option<usize>,
}

impl<'a> BlockService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
        vspace_mgr: &'a mut VSpaceManager,
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
            vspace_mgr,
            connected_client: None,
        }
    }

    pub fn setup_ring(
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
        self.vspace_mgr.map_frame(
            frame.clone(),
            RING_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace_mgr,
        )?;
        glenda::arch::sync::fence();

        let ring = unsafe {
            IoUring::new(RING_VA as *mut u8, glenda::arch::mem::PGSIZE, sq_entries, cq_entries)
        };
        let mut server = IoUringServer::new(ring);

        server.set_client_notify(notify_ep);

        if let Some(blk) = self.blk.as_mut() {
            blk.set_ring_server(server);
        } else {
            error!("setup_ring: blk is None!");
        }

        Ok(frame)
    }

    pub fn setup_shm(
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

impl<'a> BlockService<'a> {
    pub fn capacity(&self) -> u64 {
        self.blk.as_ref().map(|b| b.capacity()).unwrap_or(0)
    }

    pub fn block_size(&self) -> u32 {
        self.blk.as_ref().map(|b| b.block_size()).unwrap_or(0)
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
        loop {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply_cap.cap());
            utcb.set_recv_window(self.recv);
            match self.endpoint.recv(&mut utcb) {
                Ok(_) => {}
                Err(e) => {
                    error!("Recv error: {:?}", e);
                    continue;
                }
            };

            let badge = utcb.get_badge();
            let proto = utcb.get_msg_tag().proto();
            let label = utcb.get_msg_tag().label();

            let res = self.dispatch(&mut utcb);
            if let Err(e) = res {
                if e == Error::Success {
                    continue;
                }
                error!(
                    "Failed to dispatch message for {:#x}: {:?}, proto={:#x}, label={:#x}",
                    badge.bits(),
                    e,
                    proto,
                    label
                );
                utcb.set_msg_tag(MsgTag::err());
                utcb.set_mr(0, e as usize);
            }

            if let Err(e) = self.reply(&mut utcb) {
                error!("Reply failed: {:?}", e);
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge().bits();
        if badge != 0 && self.connected_client.is_some() && self.connected_client != Some(badge) {
            let proto = utcb.get_msg_tag().proto();
            if proto != glenda::protocol::KERNEL_PROTO {
                return Err(Error::PermissionDenied);
            }
        }

        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |u| {
                    let badge = u.get_badge();
                    let bits = badge.bits();

                    // Determine flags
                    let is_cq = bits & glenda::io::uring::NOTIFY_IO_URING_CQ != 0;
                    let is_sq = bits & glenda::io::uring::NOTIFY_IO_URING_SQ != 0;
                    let is_irq = bits & IRQ_BADGE != 0;
                    if let Some(blk) = s.blk.as_mut() {
                        if is_irq {
                            blk.handle_irq();
                            if let Some(irq) = s.irq.as_ref() {
                                irq.ack()?;
                            }
                        }
                        if is_cq || is_sq {
                            blk.handle_ring();
                        }
                    }
                    else {
                        error!("Device not initialized");
                    }
                    Ok(())
                })
            },
            (glenda::protocol::DEVICE_PROTO, label) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    use glenda::interface::device::DeviceService;
                    use glenda::protocol::device::*;
                    match label {
                        GET_MMIO => {
                            let id = u.get_mr(0);
                            let recv = u.get_recv_window();
                            let (frame, addr, size) = s.dev.get_mmio(Badge::null(), id, recv)?;
                            u.set_mr(0, addr);
                            u.set_mr(1, size);
                            Ok(frame.cap())
                        }
                        GET_IRQ => {
                            let id = u.get_mr(0);
                            let recv = u.get_recv_window();
                            let irq = s.dev.get_irq(Badge::null(), id, recv)?;
                            Ok(irq.cap())
                        }
                        _ => Err(Error::NotSupported),
                    }
                })
            },
            (BLOCK_PROTO, block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                if badge != 0 && s.connected_client.is_none() {
                    s.connected_client = Some(badge);
                }
                handle_call(u, |_| Ok(s.capacity() as usize))
            },
            (BLOCK_PROTO, block::GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.block_size() as usize))
            },
            (BLOCK_PROTO, block::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let recv_slot = s.recv;
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    // Read args into local variables before move_cap
                    let vaddr = u.get_mr(0);
                    let size = u.get_mr(1);
                    let paddr = u.get_mr(2) as u64;

                    CSPACE_CAP.move_cap(recv_slot, slot)?;

                    let frame = Frame::from(slot);
                    s.setup_shm(frame, vaddr, paddr, size)?;
                    Ok(())
                })
            },
            (BLOCK_PROTO, block::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                handle_cap_call(u, |u| {
                    let recv_slot = s.recv;
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    // Read args into local variables before move_cap
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;

                    CSPACE_CAP.move_cap(recv_slot, slot)?;

                    // The client passed its notification endpoint in the UTCB.
                    // It was moved to slot.
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
