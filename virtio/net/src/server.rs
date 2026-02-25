use crate::net::VirtIONet;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{MemoryService, ResourceService, SystemService};
use glenda::io::uring::{IoUringBuffer as IoUring, IoUringServer};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, UTCB};
use glenda::protocol::device::net::MacAddress;
use glenda::utils::manager::{CSpaceManager, CSpaceService};
use glenda_drivers::interface::{DriverService, NetDriver};
use glenda_drivers::protocol::{net, NET_PROTO};

pub struct NetService<'a> {
    pub net: Option<VirtIONet>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
    pub recv: CapPtr,
}

pub const IRQ_BADGE: Badge = Badge::new(0x1);

impl<'a> NetService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            net: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply_cap: Reply::from(CapPtr::null()),
            dev,
            res,
            cspace_mgr,
            recv: CapPtr::null(),
        }
    }
}

impl<'a> NetDriver for NetService<'a> {
    fn mac_address(&self) -> MacAddress {
        let octets = self.net.as_ref().map(|n| n.mac()).unwrap_or([0; 6]);
        MacAddress { octets }
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
        let (_, frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;

        // Map the DMA frame to our virtual address space
        self.res.mmap(
            Badge::null(),
            frame.clone(),
            crate::layout::RING_VA,
            glenda::arch::mem::PGSIZE,
        )?;
        glenda::arch::sync::fence();

        let ring = unsafe {
            IoUring::new(
                crate::layout::RING_VA as *mut u8,
                glenda::arch::mem::PGSIZE,
                sq_entries,
                cq_entries,
            )
        };
        let mut server = IoUringServer::new(ring);

        server.set_client_notify(notify_ep);

        if let Some(net) = self.net.as_mut() {
            net.set_ring_server(server);
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
        // Map SHM frame to local space (SHM_VA)
        use glenda::cap::VSPACE_CAP;
        use glenda::mem::Perms;
        VSPACE_CAP.map(frame, crate::layout::SHM_VA, Perms::READ | Perms::WRITE)?;

        if let Some(net) = self.net.as_mut() {
            // Note: net-internal shm will use local SHM_VA for access
            // but keep the Gopher's vaddr as the client_vaddr to match incoming SQEs
            net.setup_shm(frame, crate::layout::SHM_VA, paddr, size)?;
            if let Some(shm) = net.buffer.as_mut() {
                shm.set_client_vaddr(vaddr);
            }
        } else {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }
}

impl<'a> SystemService for NetService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, endpoint: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = endpoint;
        self.reply_cap = Reply::from(reply);
        self.recv = recv;
        if let Some(net) = self.net.as_mut() {
            net.set_endpoint(self.endpoint.clone());
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        let mut utcb = unsafe { UTCB::new() };
        loop {
            // Clear receive slot before receiving new messages
            let _ = self.cspace_mgr.root().delete(self.recv);
            utcb.clear();
            utcb.set_reply_window(self.reply_cap.cap());
            utcb.set_recv_window(self.recv);

            if let Err(e) = self.endpoint.recv(&mut utcb) {
                log!("Recv error: {:?}", e);
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
                    log!("Dispatch error: {:?}", e);
                    utcb.set_msg_tag(glenda::ipc::MsgTag::err());
                    utcb.set_mr(0, e as usize);
                    let _ = self.reply(&mut utcb);
                }
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |u| {
                    let badge = u.get_badge();
                    if let Some(net) = s.net.as_mut() {
                        if (badge.bits() & IRQ_BADGE.bits()) != 0 {
                            net.handle_irq();
                        } else {
                            net.handle_ring();
                        }
                    }
                    Ok(())
                })
            },
            (NET_PROTO, net::GET_MAC) => |s: &mut Self, u: &mut UTCB| {
                let mac = s.mac_address();
                handle_call(u, |u| {
                    for i in 0..6 {
                        u.set_mr(i, mac.octets[i] as usize);
                    }
                    Ok(0usize)
                })
            },
            (NET_PROTO, net::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
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
            (NET_PROTO, net::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                 handle_cap_call(u, |u| {
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;

                    let slot = s.cspace_mgr.alloc(s.res)?;
                    s.cspace_mgr.root().move_cap(s.recv, slot)?;
                    let notify_ep = Endpoint::from(slot);

                    let frame = s.setup_ring(sq, cq, notify_ep, CapPtr::null())?;
                    Ok(frame.cap())
                 })
            },
            (_, _) => |_s: &mut Self, _u: &mut UTCB| {
                Err(Error::NotSupported)
            }
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply_cap.reply(utcb)
    }

    fn stop(&mut self) {}
}
