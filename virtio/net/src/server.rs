use crate::net::VirtIONet;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, UTCB};
use glenda::mem::shm::SharedMemory;
use glenda::protocol::device::net::MacAddress;
use glenda::utils::manager::{CSpaceManager, CSpaceService};
use glenda_drivers::interface::{DriverService, NetDriver};
use glenda_drivers::io_uring::{IoRing, IoRingServer};
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
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;

        let shm =
            SharedMemory::from_frame(frame.clone(), paddr as usize, glenda::arch::mem::PGSIZE);
        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        let mut server = IoRingServer::new(ring);

        server.set_client_notify(notify_ep);
        server.set_notify_tag(glenda::ipc::MsgTag::new(
            NET_PROTO,
            net::NOTIFY_IO,
            glenda::ipc::MsgFlags::NONE,
        ));

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
        if let Some(net) = self.net.as_mut() {
            net.setup_shm(frame, vaddr, paddr, size)
        } else {
            Err(Error::NotInitialized)
        }
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
            utcb.clear();
            utcb.set_reply_window(self.reply_cap.cap());
            utcb.set_recv_window(self.recv);

            if let Err(e) = self.endpoint.recv(&mut utcb) {
                glenda::println!("NetService recv error: {:?}", e);
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
                    glenda::println!("NetService dispatch error: {:?}", e);
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
                if let Some(net) = self.net.as_mut() {
                    net.handle_irq();
                }
                Ok(())
            });
        }

        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(net) = s.net.as_mut() {
                        net.handle_irq();
                    }
                    Ok(())
                })
            },
            (NET_PROTO, net::NOTIFY_SQ) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(net) = s.net.as_mut() {
                        net.handle_ring();
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
            (_, _) => |s: &mut Self, u: &mut UTCB| {
                let tag = u.get_msg_tag();
                if tag.proto() == 0 && tag.label() == 0 {
                     return handle_notify(u, |_u| {
                        if let Some(net) = s.net.as_mut() {
                            net.handle_irq();
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
