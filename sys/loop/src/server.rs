use glenda::cap::{Endpoint, ENDPOINT_CAP, REPLY_CAP};
use glenda::error::Error;
use glenda::ipc::{MsgFlags, MsgTag, UTCB};
use glenda::protocol::{fs, FS_PROTO};
use glenda_drivers::protocol::{block, BLOCK_PROTO};

pub struct BadgedFileClient {
    endpoint: Endpoint,
    badge: usize,
}

impl BadgedFileClient {
    pub fn new(endpoint: Endpoint, badge: usize) -> Self {
        Self { endpoint, badge }
    }

    pub fn stat(&self) -> Result<glenda::protocol::fs::Stat, Error> {
        let mut utcb = unsafe { UTCB::new() };
        let tag = MsgTag::new(FS_PROTO, fs::STAT, MsgFlags::NONE);
        utcb.set_msg_tag(tag);
        utcb.set_mr(3, self.badge);

        self.endpoint.call(&mut utcb)?;
        let size = utcb.get_mr(0) as u64;
        let mode = utcb.get_mr(1) as u32;
        Ok(glenda::protocol::fs::Stat { size, mode, ..Default::default() })
    }
}

pub struct LoopBlockServer {
    file: BadgedFileClient,
}

impl LoopBlockServer {
    pub fn new(file_ep: Endpoint, badge: usize) -> Self {
        Self { file: BadgedFileClient::new(file_ep, badge) }
    }

    pub fn run(&mut self) -> Result<(), Error> {
        loop {
            let mut utcb = unsafe { UTCB::new() };
            if let Err(_) = ENDPOINT_CAP.recv(&mut utcb) {
                continue;
            }

            let res = glenda::ipc_dispatch! {
                self, utcb,
                (BLOCK_PROTO, block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                    s.dispatch_capacity(u)
                }
            };

            let tag = utcb.get_msg_tag();
            let reply_tag = match res {
                Ok(val) => {
                    utcb.set_mr(0, val);
                    MsgTag::new(tag.proto(), tag.label(), MsgFlags::NONE)
                }
                Err(e) => {
                    utcb.set_mr(0, e as usize);
                    MsgTag::new(tag.proto(), tag.label(), MsgFlags::NONE)
                }
            };
            utcb.set_msg_tag(reply_tag);
            let _ = REPLY_CAP.reply(&mut utcb);
        }
    }

    fn dispatch_capacity(&mut self, _utcb: &mut UTCB) -> Result<usize, Error> {
        let stat = self.file.stat()?;
        Ok((stat.size / 512) as usize)
    }
}
