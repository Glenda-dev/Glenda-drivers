use crate::net::VirtIONet;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, RECV_SLOT, VSPACE_CAP};
use glenda::error::Error;
use glenda::interface::device::NetDevice;
use glenda::interface::{DriverService, SystemService};
use glenda::ipc::server::handle_call;
use glenda::ipc::{MsgFlags, MsgTag, UTCB};
use glenda::mem::Perms;
use glenda::protocol as root_protocol;
use glenda::protocol::device as device_protocol;
use glenda::protocol::device::net as net_proto;
use glenda::protocol::device::DeviceNode;

pub struct NetService {
    net: Option<VirtIONet>,
    endpoint: Endpoint,
    reply: Reply,
    running: bool,
}

impl NetService {
    pub fn new() -> Self {
        Self {
            net: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            running: false,
        }
    }
}

impl DriverService for NetService {
    fn init(&mut self, node: DeviceNode) {
        log!("Initializing Net device: {}", node.id);

        let mmio_slot = 21;
        let unicorn = Endpoint::from(CapPtr::from(11));
        let tag =
            MsgTag::new(device_protocol::NET_PROTO, device_protocol::MAP_MMIO, MsgFlags::HAS_CAP);

        let mut utcb = unsafe { UTCB::new() };
        utcb.clear();
        utcb.set_msg_tag(tag);
        utcb.set_mr(0, node.id);
        utcb.set_mr(1, 0);
        utcb.set_mr(2, mmio_slot);

        unicorn.call(&mut utcb).expect("Failed request MMIO cap");

        let mmio_va = 0x6000_2000;
        VSPACE_CAP
            .map(
                Frame::from(CapPtr::from(mmio_slot)),
                mmio_va,
                Perms::READ | Perms::WRITE | Perms::USER,
            )
            .expect("Failed to map MMIO");

        let net = unsafe { VirtIONet::new(mmio_va).expect("Failed to init virtio-net") };
        self.net = Some(net);
    }
}

impl NetDevice for NetService {
    fn mac_address(&self) -> net_proto::MacAddress {
        let octets = self.net.as_ref().map(|n| n.mac()).unwrap_or([0; 6]);
        net_proto::MacAddress { octets }
    }

    fn send(&mut self, _buf: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn recv(&mut self, _buf: &mut [u8]) -> Result<usize, Error> {
        Ok(0)
    }
}

impl SystemService for NetService {
    fn init(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        log!("Net Service running...");
        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(RECV_SLOT);

            if self.endpoint.recv(&mut utcb).is_ok() {
                if let Err(e) = self.dispatch(&mut utcb) {
                    utcb.set_msg_tag(MsgTag::err());
                    utcb.set_mr(0, e as usize);
                }
                let _ = self.reply(&mut utcb);
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        glenda::ipc_dispatch! {
            self, utcb,
            (device_protocol::NET_PROTO, net_proto::GET_MAC) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let mac = s.mac_address();
                    let mac_val = u64::from_le_bytes([mac.octets[0], mac.octets[1], mac.octets[2], mac.octets[3], mac.octets[4], mac.octets[5], 0, 0]);
                    u_inner.set_mr(0, mac_val as usize);
                    Ok(())
                })
            },
            (device_protocol::NET_PROTO, net_proto::SEND) => |_s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u_inner| {
                    Ok(())
                })
            },
            (device_protocol::NET_PROTO, net_proto::RECV) => |_s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u_inner| {
                    Ok(())
                })
            },
            (root_protocol::PROCESS_PROTO, root_protocol::process::EXIT) => |s: &mut Self, _u: &mut UTCB| {
                s.running = false;
                Ok(())
            }
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
