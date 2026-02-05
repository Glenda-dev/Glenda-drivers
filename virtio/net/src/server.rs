use core::ptr::NonNull;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, VSPACE_CAP};
use glenda::error::Error;
use glenda::interface::device::DriverService;
use glenda::interface::system::SystemService;
use glenda::ipc::{Badge, MsgArgs, MsgFlags, MsgTag, UTCB};
use glenda::manager::device::DeviceNode;
use glenda::mem::Perms;
use glenda::protocol as root_protocol;
use glenda::protocol::device as device_protocol;
use glenda::protocol::device::net as net_proto;

use crate::net::VirtIONet;

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
        // Discovery and Mapping
        // Assuming we need to ask Unicorn (root bus) to map us or we map ourselves if we have permission.
        // Similar to BLK service.
        
        let mmio_va = 0x6000_2000; // Use a different address than BLK (0x6000_0000)
        // Note: Real allocation should be dynamic or managed.
        
        // Mock mapping via Unicorn if needed, or direct VSPACE map if we have the cap.
        // BLK Service did:
        // let unicorn = Endpoint::from(CapPtr::from(11));
        // ... msg ...
        // VSPACE_CAP.map(...)
        
        // We replicate the pattern:
        // Assume 'node' gives us the physical address.
        // We map it to 'mmio_va'.
        // Where do we get the frame capability for the MMIO region?
        // Unicorn should provide it or we forge it if we are root?
        // In BLK service: `CapPtr::from(mmio_slot)` was used.
        // We assume we are assigned a slot.
        // Let's assume slot 21 for Net.
        
        let mmio_slot = 21;
         let unicorn = Endpoint::from(CapPtr::from(11));
        // MAP_MMIO = 1? Check protocol. Assuming same as BLK context.
        // device_protocol::MAP_MMIO
        
        // We need device_protocol definition.
        use glenda::protocol::device as device_protocol;

        let tag = MsgTag::new(device_protocol::BLOCK_PROTO, 4, MsgFlags::HAS_CAP); // Probably GENERIC or specific proto?
        // Using BLOCK_PROTO might be wrong tag proto, but Unicorn listener likely checks label.
        // BLK used: MsgTag::new(device_protocol::BLOCK_PROTO, ...)
        
        // Let's use generic proto for init calls if possible, or assume Unicorn handles it.
        // Using 0 (GENERIC) for now or check what BLK used. 
        // BLK code: `device_protocol::BLOCK_PROTO`
        
        let args = [device_protocol::MAP_MMIO, node.id, 0, mmio_slot, 0, 0, 0, 0];
        
        // We'll mimic the BLK serivce exactly for now assuming Unicorn handles device protocol messages broadly.
        unicorn.call(tag, args).expect("Failed request MMIO cap");

        VSPACE_CAP
            .map(
                Frame::from(CapPtr::from(mmio_slot)),
                mmio_va,
                Perms::READ | Perms::WRITE | Perms::USER,
            )
            .expect("Failed to map MMIO");
            
        let mut net = unsafe {
            VirtIONet::new(mmio_va)
                .expect("Failed to init virtio-net")
        };
        
        self.net = Some(net);
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
        while self.running {
            let badge_bits = self.endpoint.recv(self.reply.cap())?;
            let badge = Badge::new(badge_bits);
            let utcb = unsafe { UTCB::get() };
            let tag = utcb.msg_tag;
            let args = utcb.mrs_regs;

            let res = self.dispatch(badge, tag.label(), tag.proto(), tag.flags(), args);
            match res {
                Ok(ret) => self.reply(0, 0, MsgFlags::OK, ret)?,
                Err(e) => self.reply(0, 0, MsgFlags::ERROR, [e as usize; 8])?,
            }
        }
        Ok(())
    }

    fn dispatch(
        &mut self,
        _badge: Badge,
        label: usize,
        proto: usize,
        _flags: MsgFlags,
        args: MsgArgs,
    ) -> Result<MsgArgs, Error> {
        let net = self.net.as_mut().ok_or(Error::NotInitialized)?;

        if proto != device_protocol::NET_PROTO {
            // Check label for GENERIC?
            return Err(Error::InvalidProtocol);
        }

        match label {
            net_proto::GET_MAC => {
                let mac = net.mac();
                // Return mac in registers. 6 bytes fit in args[0] (8 bytes) ?
                // Or split.
                // MacAddress struct is [u8; 6].
                // We packaging it into u64?
                // args[0] = mac[0..6]...
                let mac_val = u64::from_le_bytes([
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 0, 0
                ]);
                Ok([mac_val as usize, 0, 0, 0, 0, 0, 0, 0])
            }
            net_proto::SEND => {
                // args[0] = len
                // Payload in shared buf? 
                // Stub
                Ok([0; 8])
            }
            net_proto::RECV => {
                // Stub
                Ok([0; 8])
            }
            _ => Err(Error::NotImplemented),
        }
    }

    fn reply(
        &mut self,
        _label: usize,
        _proto: usize,
        flags: MsgFlags,
        msg: MsgArgs,
    ) -> Result<(), Error> {
        let utcb = unsafe { UTCB::get() };
        utcb.msg_tag = MsgTag::new(root_protocol::GENERIC_PROTO, msg.len(), flags);
        utcb.mrs_regs = msg;
        self.reply.reply(utcb.msg_tag, utcb.mrs_regs)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
