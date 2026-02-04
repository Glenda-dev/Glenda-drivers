use crate::VirtIOBlk;
use core::ptr::NonNull;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, VSPACE_CAP};
use glenda::error::Error;
use glenda::interface::device::{BlockDevice, DriverService};
use glenda::interface::system::SystemService;
use glenda::ipc::{Badge, MsgArgs, MsgFlags, MsgTag, UTCB};
use glenda::manager::device::DeviceNode;
use glenda::mem::Perms;
use glenda::protocol as root_protocol;
use glenda::protocol::device as device_protocol;
use glenda::protocol::device::block::*;

pub struct BlockService {
    blk: Option<VirtIOBlk>,
    endpoint: Endpoint,
    reply: Reply,
    running: bool,
}

impl BlockService {
    pub fn new() -> Self {
        Self {
            blk: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            running: false,
        }
    }
}

impl DriverService for BlockService {
    fn init(&mut self, node: DeviceNode) {
        // 1. Discovery and MMIO mapping (mock unicorn call)
        let unicorn = Endpoint::from(CapPtr::from(11));
        let mmio_slot = 20;

        let tag = MsgTag::new(device_protocol::BLOCK_PROTO, 4, MsgFlags::HAS_CAP);
        let args = [device_protocol::MAP_MMIO, node.id, 0, mmio_slot, 0, 0, 0, 0];
        unicorn.call(tag, args).unwrap();

        let mmio_va = 0x6000_0000;
        VSPACE_CAP
            .map(
                Frame::from(CapPtr::from(mmio_slot)),
                mmio_va,
                Perms::READ | Perms::WRITE | Perms::USER,
            )
            .expect("Failed to map MMIO");

        let mut blk = unsafe {
            VirtIOBlk::new(NonNull::new(mmio_va as *mut u8).unwrap())
                .expect("Failed to init virtio-blk")
        };
        blk.init_hardware().expect("Failed to init hardware");

        self.blk = Some(blk);
    }
}

impl SystemService for BlockService {
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
        let blk = self.blk.as_mut().ok_or(Error::NotInitialized)?;

        if proto != device_protocol::BLOCK_PROTO {
            return Err(Error::InvalidProtocol);
        }

        match label {
            GET_CAPACITY => {
                let cap = blk.capacity();
                Ok([cap as usize, (cap >> 32) as usize, 0, 0, 0, 0, 0, 0])
            }
            GET_BLOCK_SIZE => {
                let size = blk.block_size();
                Ok([size as usize, 0, 0, 0, 0, 0, 0, 0])
            }
            READ_BLOCKS => {
                let sector = args[0] as u64 | ((args[1] as u64) << 32);
                blk.read_blocks(sector, &mut [])?; // Placeholder
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
