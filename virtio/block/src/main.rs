#![no_std]
#![no_main]

extern crate alloc;

use core::ptr::NonNull;
use glenda::cap::{CapPtr, Endpoint, TCB_CAP};
use glenda::error::Error;
use glenda::interface::device::BlockDevice;
use glenda::interface::{DriverService, SystemService};
use glenda::ipc::{Badge, MsgArgs, MsgFlags, MsgTag, UTCB};
use glenda::protocol::device::block::*;
use glenda::protocol::device::BLOCK_PROTO;
use virtio_common::{consts::*, VirtIOError, VirtIOTransport};

struct VirtIOBlk {
    transport: VirtIOTransport,
    // Add other fields like virtqueues here
}

impl VirtIOBlk {
    unsafe fn new(base: NonNull<u8>) -> Result<Self, VirtIOError> {
        let transport = VirtIOTransport::new(base)?;
        Ok(Self { transport })
    }

    fn init_hardware(&mut self) -> Result<(), VirtIOError> {
        self.transport.set_status(0); // Reset
        self.transport.add_status(STATUS_ACKNOWLEDGE);
        self.transport.add_status(STATUS_DRIVER);

        let features = self.transport.get_features();
        self.transport.set_features(features);

        self.transport.add_status(STATUS_FEATURES_OK);
        if self.transport.get_status() & STATUS_FEATURES_OK == 0 {
            return Err(VirtIOError::DeviceNotFound);
        }

        // Setup queues (TODO)

        self.transport.add_status(STATUS_DRIVER_OK);
        Ok(())
    }
}

impl BlockDevice for VirtIOBlk {
    fn capacity(&self) -> u64 {
        unsafe {
            let cap_low = self.transport.read_config(0);
            let cap_high = self.transport.read_config(4);
            ((cap_high as u64) << 32) | (cap_low as u64)
        }
    }

    fn block_size(&self) -> u32 {
        512
    }

    fn read_blocks(&mut self, _sector: u64, _buf: &mut [u8]) -> Result<usize, Error> {
        // Buffer transfer implementation depending on IPC mechanism
        Ok(0)
    }

    fn write_blocks(&mut self, _sector: u64, _buf: &[u8]) -> Result<usize, Error> {
        Ok(0)
    }

    fn sync(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl SystemService for VirtIOBlk {
    fn init(&mut self) -> Result<(), Error> {
        self.init_hardware().map_err(|_| Error::DeviceError)
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr) -> Result<(), Error> {
        let mut recv_reply = reply; // Reply cap for the first call
        loop {
            let info = ep.recv(recv_reply);
            let badge = info.GetBadge();
            let label = info.GetLabel(); // MsgTag label (often opcode)
            let proto = UTCB::current().get_protocol(); // Assuming UTCB holds protocol ID
                                                        // NOTE: In some L4, protocol is not in MsgTag.
                                                        // Glenda design: Protocol ID often passed or implied by Endpoint.
                                                        // Here we assume label is the opcode within BLOCK_PROTO if endpoint is typed.
                                                        // Or we check protocol if available. For now, assuming label maps to Operation.

            // Re-constructing args from MRs
            let args: MsgArgs = UTCB::current().mrs_regs;

            // Dispatch
            // We use BLOCK_PROTO here as context, though seL4 IPC doesn't carry "proto" field natively in Tag.
            // Usually protocol is agreed upon connection.
            let res = self.dispatch(badge, label, BLOCK_PROTO, MsgFlags::NONE, args);

            match res {
                Ok(ret_args) => {
                    UTCB::current().mrs_regs = ret_args;
                    let len = ret_args.iter().take_while(|&&x| x != 0).count();

                    // Prepare reply tag
                    let tag = MsgTag::new(protocol::GENERAL, label, MsgFlags::OK);
                    // simplified len
                    // In next loop, recv will reply with this info
                    // Wait. ep.recv(reply) implicitly replies?
                    // seL4_ReplyRecv or similar.
                    // Assuming Glenda's ep.recv(reply_cap) acts as "Reply to previous, then Receive next".
                    // If so, we need to set MRs before calling recv.
                }
                Err(_) => {
                    // Send error?
                    let tag = MsgTag::new_error(label);
                }
            }
        }
    }

    fn run(&mut self) -> Result<(), Error> {
        // 1. Init
        self.init()?;

        // 2. Create/Get Endpoint to listen on.
        // In a real system, we might receive this endpoint from startup args or create and register it.
        // Assuming CapPtr::from(10) is our Service Endpoint for now (mock).
        let ep = Endpoint::from(CapPtr::from(10));
        let reply = CapPtr::from(12); // Reply capability slot

        self.listen(ep, reply)
    }

    fn dispatch(
        &mut self,
        _badge: Badge,
        label: usize,
        proto: usize,
        _flags: MsgFlags,
        args: MsgArgs,
    ) -> Result<MsgArgs, Error> {
        if proto != BLOCK_PROTO {
            return Err(Error::InvalidProto);
        }

        match label {
            GET_CAPACITY => {
                let cap = self.capacity();
                Ok([cap as usize, (cap >> 32) as usize, 0, 0, 0, 0, 0, 0])
            }
            GET_BLOCK_SIZE => {
                let size = self.block_size();
                Ok([size as usize, 0, 0, 0, 0, 0, 0, 0])
            }
            READ_BLOCKS => {
                let sector = args[0] as u64 | ((args[1] as u64) << 32);
                let _count = args[2];
                // Read into shared buffer or IPC regs?
                // For simplicity, just Ack
                Ok([0, 0, 0, 0, 0, 0, 0, 0])
            }
            WRITE_BLOCKS => Ok([0, 0, 0, 0, 0, 0, 0, 0]),
            SYNC => {
                self.sync()?;
                Ok([0, 0, 0, 0, 0, 0, 0, 0])
            }
            _ => Err(Error::InvalidOp),
        }
    }

    fn reply(
        &mut self,
        _label: usize,
        _proto: usize,
        _flags: MsgFlags,
        _msg: MsgArgs,
    ) -> Result<(), Error> {
        // Logic handled in listen loop for now
        Ok(())
    }

    fn exit() {
        TCB_CAP.exit();
    }
}

#[no_mangle]
fn main() -> usize {
    let mmio_ptr = 0x1000_1000 as *mut u8; // Mock address
    let mut driver = unsafe {
        VirtIOBlk::new(NonNull::new(mmio_ptr).unwrap()).expect("Failed to init virtio-blk")
    };

    if let Err(e) = driver.run() {
        // Log error
    }
    0
}
