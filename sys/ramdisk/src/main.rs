#![no_std]
#![no_main]

#[macro_use]
extern crate glenda;

extern crate alloc;
mod layout;

use crate::layout::{
    BUFFER_SLOT, BUFFER_VA, DEVICE_SLOT, MMIO_SLOT, MMIO_VA, NOTIFY_SLOT, RING_SLOT, RING_VA,
};
use glenda::cap::{
    CSPACE_CAP, CapPtr, CapType, ENDPOINT_CAP, ENDPOINT_SLOT, Endpoint, Frame, MONITOR_CAP,
    RECV_SLOT, REPLY_SLOT, Reply,
};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::mem::io_uring::{IORING_OP_READ, IORING_OP_SYNC, IORING_OP_WRITE, IoUringSqe};
use glenda::mem::shm::SharedMemory;
use glenda::protocol::resource::{DEVICE_ENDPOINT, ResourceType};
use glenda::protocol::{GENERIC_PROTO, generic};
use glenda_drivers::io_uring::{IoRing, IoRingServer};
use glenda_drivers::protocol::BLOCK_PROTO;

pub struct Ramdisk {
    data: &'static mut [u8],
    block_size: u32,
    ring: Option<IoRingServer>,
    buffer: Option<SharedMemory>,
}

impl Ramdisk {
    pub fn new(data: &'static mut [u8]) -> Self {
        Self { data, block_size: 4096, ring: None, buffer: None }
    }

    pub fn capacity(&self) -> u64 {
        (self.data.len() as u64) / (self.block_size as u64)
    }

    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    pub fn setup_buffer(
        &mut self,
        res: &mut ResourceClient,
        client_vaddr: usize,
        size: usize,
        paddr: u64,
        recv_slot: CapPtr,
    ) -> Result<(), Error> {
        // Move the cap from recv window to BUFFER_SLOT
        CSPACE_CAP.move_cap(recv_slot, BUFFER_SLOT)?;
        let frame = Frame::from(BUFFER_SLOT);
        let pages = (size + glenda::arch::mem::PGSIZE - 1) / glenda::arch::mem::PGSIZE;

        res.mmap(Badge::null(), frame.clone(), BUFFER_VA, pages * glenda::arch::mem::PGSIZE)?;

        // We use our own BUFFER_VA for data access, but we need to know the client's vaddr
        // to translate SQE addresses.
        let mut shm = SharedMemory::from_frame(frame, BUFFER_VA, pages * glenda::arch::mem::PGSIZE);
        shm.set_client_vaddr(client_vaddr);
        shm.set_paddr(paddr);
        self.buffer = Some(shm);
        log!(
            "SHM buffer setup: client_vaddr={:#x}, driver_vaddr={:#x}, paddr={:#x}, size={}",
            client_vaddr,
            BUFFER_VA,
            paddr,
            size
        );
        Ok(())
    }

    pub fn setup_ring(
        &mut self,
        res: &mut ResourceClient,
        sq_entries: u32,
        cq_entries: u32,
        endpoint: Endpoint,
    ) -> Result<Frame, glenda::error::Error> {
        log!("Setting up ring: SQ={}, CQ={}", sq_entries, cq_entries);
        // 1. Allocate a frame for the ring
        // Each SQE is 64 bytes, CQE is 16 bytes. Header is 64 bytes.
        // For 4 entries, we only need a few hundred bytes, so 1 page is plenty.
        let frame = Frame::from(res.alloc(Badge::null(), CapType::Frame, 1, RING_SLOT)?);
        // 2. Map it in our space
        res.mmap(Badge::null(), frame, RING_VA, glenda::arch::mem::PGSIZE)?;
        // 3. Init IoRing
        let shm = SharedMemory::from_frame(frame.clone(), RING_VA, glenda::arch::mem::PGSIZE);
        let ring = IoRing::new(shm, sq_entries, cq_entries)?;
        let mut server = IoRingServer::new(ring);
        server.set_client_notify(endpoint);
        server.set_notify_tag(MsgTag::new(
            BLOCK_PROTO,
            glenda_drivers::protocol::block::NOTIFY_IO,
            glenda::ipc::MsgFlags::NONE,
        ));
        self.ring = Some(server);
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
        let offset = sqe.off;
        let len = sqe.len as usize;
        let addr = if let Some(ref shm) = self.buffer {
            let buffer_vaddr = shm.vaddr();
            let client_vaddr = shm.client_vaddr();
            let buffer_size = shm.size();

            // Translate client address to local address
            if (sqe.addr as usize) < client_vaddr
                || (sqe.addr as usize) + len > client_vaddr + buffer_size
            {
                log!(
                    "Error: Client address {:#x} out of SHM boundary [{:#x}, {:#x})",
                    sqe.addr,
                    client_vaddr,
                    client_vaddr + buffer_size
                );
                return -(Error::InvalidArgs as i32);
            }
            (buffer_vaddr + (sqe.addr as usize - client_vaddr)) as *mut u8
        } else {
            sqe.addr as *mut u8
        };

        log!(
            "Processing SQE: opcode={}, offset={}, len={}, addr={:?}",
            sqe.opcode,
            offset,
            len,
            addr
        );

        if offset + len as u64 > self.data.len() as u64 {
            return -(Error::InvalidArgs as i32);
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
        log!("Got memory region: paddr={:#x}, size={:#x}", paddr, size);

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
            name: alloc::string::String::from("ramdisk"),
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
                match SystemService::dispatch(self, &mut utcb) {
                    Ok(_) => {
                        let _ = SystemService::reply(self, &mut utcb);
                    }
                    Err(Error::Success) => {
                        // Successfully handled, but no reply needed (e.g., notification)
                    }
                    Err(e) => {
                        let badge = utcb.get_badge();
                        let tag = utcb.get_msg_tag();
                        error!(
                            "Dispatch error: {:?} badge={}, proto={:#x}, label={:#x}",
                            e,
                            badge,
                            tag.proto(),
                            tag.label()
                        );
                        utcb.set_msg_tag(MsgTag::err());
                        utcb.set_mr(0, e as usize);
                        let _ = SystemService::reply(self, &mut utcb);
                    }
                }
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        glenda::ipc_dispatch! {
            self, utcb,
            (GENERIC_PROTO, generic::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(ramdisk) = s.ramdisk.as_mut() {
                        ramdisk.handle_io()?;
                    }
                    Ok(())
                })
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.ramdisk.as_ref().unwrap().capacity() as usize))
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.ramdisk.as_ref().unwrap().block_size() as usize))
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let client_vaddr = u.get_mr(0);
                    let size = u.get_mr(1);
                    let paddr = u.get_mr(2) as u64;
                    let res = unsafe { &mut *(s.res as *mut ResourceClient) };
                    s.ramdisk.as_mut().unwrap().setup_buffer(res, client_vaddr, size, paddr, s.recv)?;
                    Ok(0usize)
                })
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                handle_cap_call(u, |u| {
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;
                    // Transfer notification endpoint
                    let res = unsafe { &mut *(s.res as *mut ResourceClient) };
                    // Move the cap from recv window to NOTIFY_SLOT
                    CSPACE_CAP.move_cap(s.recv, NOTIFY_SLOT)?;
                    let notify_ep = Endpoint::from(NOTIFY_SLOT);

                    let ramdisk = s.ramdisk.as_mut().unwrap();
                    let frame = ramdisk.setup_ring(res, sq, cq, notify_ep)?;
                    Ok(frame.cap())
                })
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::NOTIFY_SQ) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_u| {
                    if let Some(ramdisk) = s.ramdisk.as_mut() {
                        ramdisk.handle_io()?;
                    }
                    Ok(())
                })
            },
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_u| {
                    if let Some(ramdisk) = s.ramdisk.as_mut() {
                        ramdisk.handle_io()?;
                    }
                    Ok(())
                })
            },
            (_, _) => |_, _| {
                Err(Error::NotSupported)
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

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("Ramdisk");
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
