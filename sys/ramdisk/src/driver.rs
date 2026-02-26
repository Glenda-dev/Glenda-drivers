use crate::layout::{BUFFER_SLOT, BUFFER_VA, RING_SLOT, RING_VA};
use glenda::cap::{CapType, Endpoint, Frame};
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::interface::{MemoryService, ResourceService};
use glenda::io::uring::{IOURING_OP_READ, IOURING_OP_SYNC, IOURING_OP_WRITE, IoUringSqe};
use glenda::io::uring::{IoUringBuffer as IoUring, IoUringServer};
use glenda::ipc::Badge;
use glenda::mem::shm::SharedMemory;

pub struct Ramdisk {
    data: &'static mut [u8],
    block_size: u32,
    ring: Option<IoUringServer>,
    buffer: Option<SharedMemory>,
}

impl Ramdisk {
    pub fn new(data: &'static mut [u8]) -> Self {
        Self { data, block_size: 512, ring: None, buffer: None }
    }

    pub fn capacity(&self) -> u64 {
        (self.data.len() as u64) / (self.block_size as u64)
    }

    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    pub fn set_block_size(&mut self, block_size: u32) {
        self.block_size = block_size;
    }

    pub fn setup_buffer(
        &mut self,
        res: &mut ResourceClient,
        client_vaddr: usize,
        size: usize,
        paddr: u64,
    ) -> Result<(), Error> {
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
        // 3. Init IoUring
        let ring = unsafe {
            IoUring::new(RING_VA as *mut u8, glenda::arch::mem::PGSIZE, sq_entries, cq_entries)
        };
        let mut server = IoUringServer::new(ring);
        server.set_client_notify(endpoint);
        self.ring = Some(server);
        Ok(frame)
    }

    pub fn handle_io(&mut self) -> Result<(), Error> {
        loop {
            let sqe =
                if let Some(ref mut server) = self.ring { server.next_request() } else { None };

            if let Some(sqe) = sqe {
                let res_val = self.process_sqe(&sqe);
                if let Some(ref mut server) = self.ring {
                    server.complete(sqe.user_data, res_val)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn process_sqe(&mut self, sqe: &IoUringSqe) -> i32 {
        let block_size = self.block_size();
        if sqe.off % block_size as u64 != 0 || sqe.len % block_size != 0 {
            error!(
                "Ramdisk: request not aligned to block size ({}): offset={:#x}, len={}",
                block_size, sqe.off, sqe.len
            );
            return -(Error::InvalidArgs as i32);
        }

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
                error!(
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
            IOURING_OP_READ => {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        self.data.as_ptr().add(offset as usize),
                        addr,
                        len,
                    );
                }
                len as i32
            }
            IOURING_OP_WRITE => {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        addr,
                        self.data.as_mut_ptr().add(offset as usize),
                        len,
                    );
                }
                len as i32
            }
            IOURING_OP_SYNC => 0,
            _ => -(glenda::error::Error::NotSupported as i32),
        }
    }
}
