use crate::VirtIOBlk;
use core::ptr::NonNull;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, VSPACE_CAP, RECV_SLOT};
use glenda::error::Error;
use glenda::interface::device::BlockDevice;
use glenda::interface::{DriverService, SystemService};
use glenda::ipc::server::handle_call;
use glenda::ipc::{MsgFlags, MsgTag, UTCB};
use glenda::protocol::device::DeviceNode;
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
        // 1. Discovery and MMIO mapping
        let unicorn = Endpoint::from(CapPtr::from(11));
        let mmio_slot = 20;

        let tag = MsgTag::new(device_protocol::BLOCK_PROTO, device_protocol::MAP_MMIO, MsgFlags::HAS_CAP);
        let mut utcb = unsafe { UTCB::new() };
        utcb.clear();
        utcb.set_msg_tag(tag);
        utcb.set_mr(0, node.id);
        utcb.set_mr(1, 0);
        utcb.set_mr(2, mmio_slot);
        
        unicorn.call(&mut utcb).expect("Failed to call unicorn");

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

impl BlockDevice for BlockService {
    fn capacity(&self) -> u64 {
        self.blk.as_ref().map(|b| b.capacity()).unwrap_or(0)
    }

    fn block_size(&self) -> u32 {
        self.blk.as_ref().map(|b| b.block_size()).unwrap_or(0)
    }

    fn read_blocks(&mut self, sector: u64, buf: &mut [u8]) -> Result<usize, Error> {
        self.blk
            .as_mut()
            .ok_or(Error::NotInitialized)?
            .read_blocks(sector, buf)
    }

    fn write_blocks(&mut self, sector: u64, buf: &[u8]) -> Result<usize, Error> {
        self.blk
            .as_mut()
            .ok_or(Error::NotInitialized)?
            .write_blocks(sector, buf)
    }

    fn sync(&mut self) -> Result<(), Error> {
        self.blk.as_mut().ok_or(Error::NotInitialized)?.sync()
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
        log!("Block Service running...");
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
            (device_protocol::BLOCK_PROTO, GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let cap = s.capacity();
                    u_inner.set_mr(0, cap as usize);
                    u_inner.set_mr(1, (cap >> 32) as usize);
                    Ok(())
                })
            },
            (device_protocol::BLOCK_PROTO, GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let size = s.block_size();
                    u_inner.set_mr(0, size as usize);
                    Ok(())
                })
            },
            (device_protocol::BLOCK_PROTO, READ_BLOCKS) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let sector = u_inner.get_mr(0) as u64 | ((u_inner.get_mr(1) as u64) << 32);
                    s.read_blocks(sector, &mut [])?; // Placeholder
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

