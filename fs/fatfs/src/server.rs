use alloc::vec::Vec;
use core::slice;
use log::{debug, error};

use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::error::Error;
use glenda::interface::fs::{FileHandleService, FileSystemService};
use glenda::interface::system::SystemService;
use glenda::ipc::{Badge, MsgArgs, MsgFlags, MsgTag, UTCB};
use glenda::protocol::fs::{self as fs_proto, DEntry, OpenFlags, Stat};

use crate::fs::FatFs;

pub struct FatFsService {
    fs: Option<FatFs>,
    endpoint: Endpoint,
    reply: Reply,
    running: bool,
}

impl FatFsService {
    pub fn new() -> Self {
        Self {
            fs: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            running: false,
        }
    }

    pub fn init_fs(&mut self, block_device: Endpoint) {
        // Initialize FatFs with the block device
        self.fs = Some(FatFs::new(block_device));
    }
}

impl SystemService for FatFsService {
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
        let fs = self.fs.as_mut().ok_or(Error::NotInitialized)?;

        if proto != fs_proto::PROTOCOL_ID {
            return Err(Error::InvalidProtocol);
        }

        match label {
            fs_proto::OPEN => {
                // args: [flags, mode], string: path -> cap: handle
                // TODO: Read path from UTCB buffer
                let flags = OpenFlags::from_bits_truncate(args[0]);
                let mode = args[1] as u32;

                // Only mock path reading for now as we don't have helper to read string from UTCB easily here
                // Assumed path is in message buffer
                let path = "mock_path"; // Helper needed to extract path

                let cap = fs.open(path, flags, mode)?;
                // Send cap back? Currently open returns usize (CapPtr val)
                Ok([cap, 0, 0, 0, 0, 0, 0, 0])
            }
            fs_proto::MKDIR => {
                let mode = args[0] as u32;
                let path = "mock_path";
                fs.mkdir(path, mode)?;
                Ok([0; 8])
            }
            fs_proto::UNLINK => {
                let path = "mock_path";
                fs.unlink(path)?;
                Ok([0; 8])
            }
            // ... Implement other dispatch methods
            _ => Err(Error::InvalidMethod),
        }
    }

    fn reply(
        &mut self,
        label: usize,
        proto: usize,
        flags: MsgFlags,
        msg: MsgArgs,
    ) -> Result<(), Error> {
        let tag = MsgTag::new(proto, label, flags);
        self.reply.reply(tag, msg)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
