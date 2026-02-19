use crate::log;
use crate::net::VirtIONet;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call};
use glenda::ipc::{Badge, UTCB};
use glenda::mem::shm::SharedMemory;
use glenda::protocol;
use glenda::protocol::device::net::MacAddress;
use glenda::utils::manager::{CSpaceManager, CSpaceService};
use glenda_drivers::interface::{DriverService, NetDriver};
use glenda_drivers::io_uring::{IoRing, IoRingServer};
use glenda_drivers::protocol::{NET_PROTO, net};

pub struct NetService<'a> {
    pub net: Option<VirtIONet>,
    pub endpoint: Endpoint,
    pub reply_cap: Reply,
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
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
    ) -> Result<Frame, Error> {
        let slot = self.cspace_mgr.alloc(self.res)?;
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, slot)?;

        let shm = SharedMemory::from_frame(frame.clone(), paddr as usize, 16384);
        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        let mut server = IoRingServer::new(ring);

        server.set_client_notify(self.endpoint.clone());

        if let Some(net) = self.net.as_mut() {
            net.set_ring_server(server);
        }

        Ok(frame)
    }
}

impl<'a> SystemService for NetService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, endpoint: Endpoint, reply: CapPtr, _recv: CapPtr) -> Result<(), Error> {
        self.endpoint = endpoint;
        self.reply_cap = Reply::from(reply);
        if let Some(net) = self.net.as_mut() {
            net.set_endpoint(self.endpoint.clone());
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        let mut utcb = unsafe { UTCB::new() };
        loop {
            if let Err(e) = self.endpoint.recv(&mut utcb) {
                glenda::println!("NetService recv error: {:?}", e);
                continue;
            }
            if let Err(e) = self.dispatch(&mut utcb) {
                glenda::println!("NetService dispatch error: {:?}", e);
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge();

        if badge & IRQ_BADGE != Badge::null() {
            if let Some(net) = self.net.as_mut() {
                net.handle_irq();
            }
            return Ok(());
        }

        let tag = utcb.get_msg_tag();
        if tag.proto() == 0 {
            if let Some(net) = self.net.as_mut() {
                net.handle_ring();
            }
            return Ok(());
        }

        glenda::ipc_dispatch! {
            self, utcb,
            (NET_PROTO, net::GET_MAC) => |s: &mut Self, u: &mut UTCB| {
                let mac = s.mac_address();
                handle_call(u, |u| {
                    for i in 0..6 {
                        u.set_mr(i, mac.octets[i] as usize);
                    }
                    Ok(0usize)
                })
            },
            (NET_PROTO, net::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
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
