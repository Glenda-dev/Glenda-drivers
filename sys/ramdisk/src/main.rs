#![no_std]
#![no_main]

extern crate alloc;
mod layout;

use crate::layout::{DEVICE_SLOT, MMIO_SLOT, MMIO_VA, RING_SLOT, RING_VA};
use glenda::cap::{
    CapPtr, CapType, ENDPOINT_CAP, ENDPOINT_SLOT, Endpoint, Frame, MONITOR_CAP, RECV_SLOT,
    REPLY_SLOT, Reply,
};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, ResourceService, SystemService};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::mem::io_uring::{IORING_OP_READ, IORING_OP_SYNC, IORING_OP_WRITE, IoUringSqe};
use glenda::mem::shm::SharedMemory;
use glenda::protocol::resource::{DEVICE_ENDPOINT, ResourceType};
use glenda_drivers::io_uring::{IoRing, IoRingServer};
use glenda_drivers::protocol::BLOCK_PROTO;

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("{}Ramdisk: {}{}", glenda::console::ANSI_BLUE, format_args!($($arg)*), glenda::console::ANSI_RESET);
    })
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => ({
        glenda::println!("{}Ramdisk: {}{}", glenda::console::ANSI_RED, format_args!($($arg)*), glenda::console::ANSI_RESET);
    })
}

pub struct Ramdisk {
    data: &'static mut [u8],
    block_size: u32,
    ring: Option<IoRingServer>,
}

impl Ramdisk {
    pub fn new(data: &'static mut [u8]) -> Self {
        Self { data, block_size: 512, ring: None }
    }

    pub fn capacity(&self) -> u64 {
        (self.data.len() as u64) / (self.block_size as u64)
    }

    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    pub fn setup_ring(
        &mut self,
        res: &mut ResourceClient,
        sq_entries: u32,
        cq_entries: u32,
    ) -> Result<Frame, glenda::error::Error> {
        log!("Setting up ring: SQ={}, CQ={}", sq_entries, cq_entries);
        // 1. Allocate a frame for the ring
        // Each SQE is 64 bytes, CQE is 16 bytes. Header is 64 bytes.
        // For 128 entries, we need ~10KB, so 4 pages (16KB) is safe.
        let frame = Frame::from(res.alloc(Badge::null(), CapType::Frame, 4, RING_SLOT)?);

        // 2. Map it in our space
        res.mmap(Badge::null(), frame, RING_VA, 4 * glenda::arch::mem::PGSIZE)?;

        // 3. Init IoRing
        let shm = SharedMemory::from_frame(frame, RING_VA, 4 * glenda::arch::mem::PGSIZE);
        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        self.ring = Some(IoRingServer::new(ring));

        Ok(frame)
    }

    pub fn handle_io(&mut self) -> Result<(), Error> {
        loop {
            let sqe = if let Some(ref server) = self.ring { server.next_request() } else { None };

            if let Some(sqe) = sqe {
                let res_val = self.process_sqe(&sqe);
                if let Some(ref server) = self.ring {
                    server.complete(sqe.user_data, res_val)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn process_sqe(&mut self, sqe: &IoUringSqe) -> i32 {
        let offset = sqe.off * self.block_size as u64;
        let len = sqe.len as usize;
        let addr = sqe.addr as *mut u8;

        if offset + len as u64 > self.data.len() as u64 {
            return -(glenda::error::Error::InvalidArgs as i32);
        }

        match sqe.opcode {
            IORING_OP_READ => {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        self.data.as_ptr().add(offset as usize),
                        addr,
                        len,
                    );
                }
                len as i32
            }
            IORING_OP_WRITE => {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        addr,
                        self.data.as_mut_ptr().add(offset as usize),
                        len,
                    );
                }
                len as i32
            }
            IORING_OP_SYNC => 0,
            _ => -(glenda::error::Error::NotSupported as i32),
        }
    }

    pub fn dispatch(&mut self, utcb: &mut UTCB, res: &mut ResourceClient) -> Result<(), Error> {
        let tag = utcb.get_msg_tag();
        match tag.label() {
            glenda_drivers::protocol::block::GET_CAPACITY => {
                utcb.set_mr(0, self.capacity() as usize);
                Ok(())
            }
            glenda_drivers::protocol::block::GET_BLOCK_SIZE => {
                utcb.set_mr(0, self.block_size as usize);
                Ok(())
            }
            glenda_drivers::protocol::block::SETUP_RING => {
                let sq_entries = utcb.get_mr(0) as u32;
                let cq_entries = utcb.get_mr(1) as u32;
                let frame = self.setup_ring(res, sq_entries, cq_entries)?;
                utcb.set_cap_transfer(frame.cap());
                Ok(())
            }
            glenda_drivers::protocol::block::NOTIFY_SQ => self.handle_io(),
            _ => Err(glenda::error::Error::NotSupported),
        }
    }
}

pub struct RamdiskService<'a> {
    ramdisk: Option<Ramdisk>,
    endpoint: Endpoint,
    reply: Reply,
    recv: CapPtr,
    running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
}

impl<'a> RamdiskService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            ramdisk: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }
}

impl<'a> SystemService for RamdiskService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap (backing store)
        utcb.set_recv_window(MMIO_SLOT);
        let (mmio, paddr, size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got memory region: paddr=0x{:x}, size=0x{:x}", paddr, size);

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, size)?;

        // 3. Init hardware (ramdisk logic)
        let data = unsafe { core::slice::from_raw_parts_mut(MMIO_VA as *mut u8, size) };
        let ramdisk = Ramdisk::new(data);
        log!(
            "Initialized Ramdisk with {} blocks ({} bytes each)",
            ramdisk.capacity(),
            ramdisk.block_size()
        );
        self.ramdisk = Some(ramdisk);

        // 4. Register logical device to Unicorn
        let desc = glenda::protocol::device::LogicDeviceDesc {
            dev_type: glenda::protocol::device::LogicDeviceType::RawBlock(size as u64),
            parent_name: alloc::string::String::from("ramdisk"),
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, ENDPOINT_SLOT)?;

        log!("Driver initialized!");
        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        log!("Listening for requests...");

        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(RECV_SLOT);

            if self.endpoint.recv(&mut utcb).is_ok() {
                if let Err(e) = SystemService::dispatch(self, &mut utcb) {
                    error!("Dispatch error: {:?}", e);
                    utcb.set_msg_tag(MsgTag::err());
                    utcb.set_mr(0, e as usize);
                    let _ = SystemService::reply(self, &mut utcb);
                } else {
                    // Only reply if it's NOT a notification (no protocol or specific label)
                    let tag = utcb.get_msg_tag();
                    if tag.proto() != 0 || tag.label() != 0 {
                        let _ = SystemService::reply(self, &mut utcb);
                    }
                }
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let tag = utcb.get_msg_tag();
        if tag.proto() == BLOCK_PROTO {
            let res = unsafe { &mut *(self.res as *mut ResourceClient) };
            if let Some(ramdisk) = self.ramdisk.as_mut() {
                ramdisk.dispatch(utcb, res)?;
            }
        } else {
            // Check for IO even if it's a generic notification
            if let Some(ramdisk) = self.ramdisk.as_mut() {
                ramdisk.handle_io()?;
            }
        }
        Ok(())
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}

#[unsafe(no_mangle)]
fn main() -> usize {
    log!("Starting Ramdisk driver...");

    let mut res_client = ResourceClient::new(MONITOR_CAP);
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get device endpoint cap");
    let mut dev_client = DeviceClient::new(Endpoint::from(DEVICE_SLOT));

    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");

    let mut service = RamdiskService::new(&mut dev_client, &mut res_client);
    service.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    if let Err(e) = SystemService::init(&mut service) {
        error!("Failed to initialize: {:?}", e);
        return 1;
    }

    if let Err(e) = service.run() {
        error!("Exited with error: {:?}", e);
        return 1;
    }
    0
}
