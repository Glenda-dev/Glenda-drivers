use crate::VirtIOBlk;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::DeviceClient;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::interface::drivers::BlockDriver;
use glenda::interface::{DriverService, SystemService};
use glenda::ipc::server::handle_call;
use glenda::ipc::{MsgTag, UTCB};
use glenda::protocol::drivers::block::{GET_BLOCK_SIZE, GET_CAPACITY, READ_BLOCKS};
use glenda::protocol::{process::EXIT, PROCESS_PROTO};

pub struct BlockService<'a> {
    pub blk: Option<VirtIOBlk>,
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
}

impl<'a> BlockService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            blk: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }
}

impl<'a> BlockDriver for BlockService<'a> {
    fn capacity(&self) -> u64 {
        self.blk.as_ref().map(|b| b.capacity()).unwrap_or(0)
    }

    fn block_size(&self) -> u32 {
        self.blk.as_ref().map(|b| b.block_size()).unwrap_or(512)
    }

    fn read_blocks(&mut self, sector: u64, buf: &mut [u8]) -> Result<usize, Error> {
        if let Some(blk) = &mut self.blk {
            blk.read_blocks(sector, buf)
        } else {
            Err(Error::Unknown)
        }
    }

    fn write_blocks(&mut self, sector: u64, buf: &[u8]) -> Result<usize, Error> {
        if let Some(blk) = &mut self.blk {
            blk.write_blocks(sector, buf)
        } else {
            Err(Error::Unknown)
        }
    }

    fn sync(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl<'a> SystemService for BlockService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        log!("Block Service running...");
        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(self.recv);

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
            (glenda::protocol::drivers::BLOCK_PROTO, GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let cap = s.capacity();
                    u_inner.set_mr(0, cap as usize);
                    u_inner.set_mr(1, (cap >> 32) as usize);
                    Ok(())
                })
            },
            (glenda::protocol::drivers::BLOCK_PROTO, GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let size = s.block_size();
                    u_inner.set_mr(0, size as usize);
                    Ok(())
                })
            },
            (glenda::protocol::drivers::BLOCK_PROTO, READ_BLOCKS) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u_inner| {
                    let sector = u_inner.get_mr(0) as u64 | ((u_inner.get_mr(1) as u64) << 32);
                    let len = u_inner.get_mr(2);
                    let mut buf = alloc::vec![0u8; len];
                    s.read_blocks(sector, &mut buf)?;
                    Ok(())
                })
            },
            (PROCESS_PROTO, EXIT) => |s: &mut Self, _u: &mut UTCB| {
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
