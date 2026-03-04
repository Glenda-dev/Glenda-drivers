use crate::layout::SHM_VA;
use crate::net::VirtIONet;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, CSPACE_CAP};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::drivers::interface::{DriverService, NetDriver};
use glenda::drivers::protocol::net::MacAddress;
use glenda::drivers::protocol::{net, NET_PROTO};
use glenda::error::Error;
use glenda::interface::{CSpaceService, ResourceService, SystemService, VSpaceService};
use glenda::io::uring::{IoUringBuffer as IoUring, IoUringServer};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct NetService<'a> {
    pub net: Option<VirtIONet>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
    pub vspace_mgr: &'a mut VSpaceManager,
    pub recv: CapPtr,
    pub connected_client: Option<usize>,
}

pub const IRQ_BADGE: Badge = Badge::new(0x1);

impl<'a> NetService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
        vspace_mgr: &'a mut VSpaceManager,
    ) -> Self {
        Self {
            net: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply_cap: Reply::from(CapPtr::null()),
            dev,
            res,
            cspace_mgr,
            vspace_mgr,
            recv: CapPtr::null(),
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
        let (_, frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;

        // Map the DMA frame to our virtual address space
        self.vspace_mgr.map_frame(
            frame.clone(),
            crate::layout::RING_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace_mgr,
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

    pub fn setup_shm(
        &mut self,
        frame: Frame,
        vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> Result<(), Error> {
        self.vspace_mgr.map_frame(
            frame.clone(),
            SHM_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            size / 4096,
            self.res,
            self.cspace_mgr,
        )?;

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

impl<'a> NetDriver for NetService<'a> {
    fn mac_address(&self) -> MacAddress {
        let octets = self.net.as_ref().map(|n| n.mac()).unwrap_or([0; 6]);
        MacAddress { octets }
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
        // Clear receive slot before receiving new messages
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
                    "Failed to dispatch message for {}: {:?}, proto={:#x}, label={:#x}",
                    badge, e, proto, label
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
            (NET_PROTO, net::GET_MAC) => |s: &mut Self, u: &mut UTCB| {
                if badge != 0 && s.connected_client.is_none() {
                    s.connected_client = Some(badge);
                }
                let mac = s.mac_address();
                handle_call(u, |u| {
                    for i in 0..6 {
                        u.set_mr(i, mac.octets[i] as usize);
                    }
                    Ok(())
                })
            },
            (NET_PROTO, net::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let vaddr = u.get_mr(0);
                    let size = u.get_mr(1);
                    let paddr = u.get_mr(2) as u64;
                    // Move the cap from recv window to a temporary slot
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    CSPACE_CAP.move_cap(s.recv, slot)?;
                    let frame = Frame::from(slot);
                    s.setup_shm(frame, vaddr, paddr, size)?;
                    Ok(())
                })
            },
            (NET_PROTO, net::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                 handle_cap_call(u, |u| {
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;

                    let slot = s.cspace_mgr.alloc(s.res)?;
                    CSPACE_CAP.move_cap(s.recv, slot)?;
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
