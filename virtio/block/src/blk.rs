use glenda::cap::{Endpoint, Frame};
use glenda::error::Error;
use glenda::mem::io_uring;
use glenda::mem::shm::SharedMemory;
use glenda_drivers::io_uring::IoRingServer;
use virtio_common::consts::*;
use virtio_common::queue::*;
use virtio_common::VirtIOTransport;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct VirtIOBlkReq {
    pub type_: u32,
    pub reserved: u32,
    pub sector: u64,
}

pub const VIRTIO_BLK_T_IN: u32 = 0;
pub const VIRTIO_BLK_T_OUT: u32 = 1;
pub const VIRTIO_BLK_T_FLUSH: u32 = 4;

pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

pub const VIRTIO_F_RING_PACKED: u64 = 1 << 34;
pub const VIRTIO_F_EVENT_IDX: u64 = 1 << 29;

pub struct VirtIOBlk {
    pub transport: VirtIOTransport,
    pub queue: Option<VirtQueue>,
    pub dma_vaddr: *mut u8,
    pub dma_paddr: u64,
    pub pending_info: [Option<(u64, u16)>; 64],
    pub ring_server: Option<IoRingServer>,
    pub endpoint: Option<Endpoint>,
    pub buffer: Option<SharedMemory>,
}

impl VirtIOBlk {
    pub fn new(transport: VirtIOTransport) -> Self {
        Self {
            transport,
            queue: None,
            dma_vaddr: core::ptr::null_mut(),
            dma_paddr: 0,
            pending_info: [None; 64],
            ring_server: None,
            endpoint: None,
            buffer: None,
        }
    }

    pub fn setup_shm(
        &mut self,
        _frame: Frame,
        vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> Result<(), Error> {
        // Since we don't map it here (it's already handled by the service layer if needed),
        // we just store the addresses.
        // Actually, the service layer didn't map it for us yet.
        // But for VirtIO, we mainly need the paddr.
        let mut shm = SharedMemory::from_frame(_frame, vaddr, size);
        shm.set_client_vaddr(vaddr);
        shm.set_paddr(paddr);
        self.buffer = Some(shm);
        log!("VirtIOBlk SHM setup: client_vaddr={:#x}, paddr={:#x}, size={}", vaddr, paddr, size);
        Ok(())
    }

    pub fn init(
        &mut self,
        dma_vaddr: *mut u8,
        dma_paddr: u64,
        endpoint: Endpoint,
    ) -> Result<(), Error> {
        self.dma_vaddr = dma_vaddr;
        self.dma_paddr = dma_paddr;
        self.endpoint = Some(endpoint);

        self.transport.set_status(0);
        self.transport.set_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        let mut features = self.transport.get_device_features();
        features &= !(VIRTIO_F_EVENT_IDX | VIRTIO_F_RING_PACKED);
        self.transport.set_driver_features(features);
        self.transport.set_status(self.transport.get_status() | STATUS_FEATURES_OK);

        let queue_paddr = dma_paddr + 8192;
        let queue_vaddr = unsafe { dma_vaddr.add(8192) };
        let queue = unsafe { VirtQueue::new(0, 128, queue_paddr, queue_vaddr) };

        unsafe { self.transport.setup_queue(&queue) };
        self.queue = Some(queue);

        self.transport.set_status(self.transport.get_status() | STATUS_DRIVER_OK);
        Ok(())
    }

    pub fn handle_irq(&mut self) {
        if self.transport.interrupt_ack() {
            self.pop_completions();
        }
    }

    pub fn handle_ring(&mut self) {
        let mut sqes = [io_uring::IoUringSqe::default(); 16];
        let mut count = 0;

        if let Some(server) = self.ring_server.as_mut() {
            while count < 16 {
                if let Some(sqe) = server.next_request() {
                    sqes[count] = sqe;
                    count += 1;
                } else {
                    break;
                }
            }
        } else {
            error!("VirtIO-Blk: handle_ring called but ring_server is None!");
            return;
        }

        if count > 0 {
            log!("Processing {} SQEs", count);
        }

        for i in 0..count {
            let sqe = sqes[i];
            if let Err(e) = self.submit_virtio_request(sqe) {
                error!("VirtIO-Blk: submit_virtio_request failed: {:?}", e);
                if let Some(server) = self.ring_server.as_mut() {
                    let _ = server.complete(sqe.user_data, -1);
                }
            }
        }

        if count > 0 {
            self.pop_completions();
        }
    }

    fn submit_virtio_request(&mut self, sqe: io_uring::IoUringSqe) -> Result<(), Error> {
        log!(
            "VirtIO-Blk: submit_virtio_request START: opcode={}, addr={:#x}, len={}",
            sqe.opcode,
            sqe.addr,
            sqe.len
        );
        let queue = self.queue.as_mut().ok_or(Error::NotInitialized)?;

        let req_idx =
            self.pending_info.iter().position(|x| x.is_none()).ok_or(Error::OutOfMemory)?;

        if self.dma_vaddr.is_null() {
            error!("VirtIO-Blk: dma_vaddr is NULL!");
            return Err(Error::NotInitialized);
        }

        let req_ptr = unsafe { (self.dma_vaddr as *mut VirtIOBlkReq).add(req_idx) };
        let status_ptr =
            unsafe { self.dma_vaddr.add(64 * core::mem::size_of::<VirtIOBlkReq>() + req_idx) };

        log!("req_ptr={:p}, status_ptr={:p}", req_ptr, status_ptr);

        let (virtio_type, is_write) = match sqe.opcode {
            io_uring::IORING_OP_READ => (VIRTIO_BLK_T_IN, false),
            io_uring::IORING_OP_WRITE => (VIRTIO_BLK_T_OUT, true),
            _ => return Err(Error::NotSupported),
        };

        unsafe {
            core::ptr::addr_of_mut!((*req_ptr).type_).write_volatile(virtio_type);
            core::ptr::addr_of_mut!((*req_ptr).reserved).write_volatile(0);
            core::ptr::addr_of_mut!((*req_ptr).sector).write_volatile(sqe.off / 512);
            status_ptr.write_volatile(0xFF);
        }

        let req_paddr = self.dma_paddr + (req_idx * core::mem::size_of::<VirtIOBlkReq>()) as u64;
        let status_paddr =
            self.dma_paddr + (64 * core::mem::size_of::<VirtIOBlkReq>() + req_idx) as u64;

        let data_paddr = if let Some(ref shm) = self.buffer {
            let client_vaddr = shm.client_vaddr();
            let paddr = shm.paddr();
            let size = shm.size();
            if (sqe.addr as usize) < client_vaddr
                || (sqe.addr as usize) + sqe.len as usize > client_vaddr + size
            {
                log!("VirtIO-Blk error: Address {:#x} out of SHM boundary", sqe.addr);
                return Err(Error::InvalidArgs);
            }
            paddr + (sqe.addr as u64 - client_vaddr as u64)
        } else {
            // Fallback to absolute paddr if no SHM (risky, but was previous behavior)
            sqe.addr
        };

        log!(
            "Submitting VirtIO request: opcode={}, paddr={:#x}, len={}",
            sqe.opcode,
            data_paddr,
            sqe.len
        );

        let d1 = queue.alloc_desc().ok_or(Error::OutOfMemory)?;
        let d2 = queue.alloc_desc().ok_or(Error::OutOfMemory)?;
        let d3 = queue.alloc_desc().ok_or(Error::OutOfMemory)?;

        queue.write_desc(
            d1,
            Descriptor {
                addr: req_paddr,
                len: core::mem::size_of::<VirtIOBlkReq>() as u32,
                flags: DESC_F_NEXT,
                next: d2,
            },
        );

        let flags = if is_write { DESC_F_NEXT } else { DESC_F_NEXT | DESC_F_WRITE };
        queue.write_desc(d2, Descriptor { addr: data_paddr, len: sqe.len, flags, next: d3 });

        queue.write_desc(
            d3,
            Descriptor { addr: status_paddr, len: 1, flags: DESC_F_WRITE, next: 0 },
        );

        glenda::arch::sync::fence();
        queue.submit(d1);
        self.pending_info[req_idx] = Some((sqe.user_data, d1));
        self.transport.notify_queue(0);

        Ok(())
    }

    fn pop_completions(&mut self) {
        if let Some(queue) = self.queue.as_mut() {
            while let Some((id, _len)) = queue.pop() {
                if let Some(pos) = self
                    .pending_info
                    .iter()
                    .position(|info| info.map_or(false, |(_, head)| head == id as u16))
                {
                    let (user_data, head) = self.pending_info[pos].take().unwrap();

                    let mut curr = head;
                    loop {
                        let next = queue.desc_table()[curr as usize].next;
                        let flags = queue.desc_table()[curr as usize].flags;
                        queue.free_desc(curr);
                        if flags & DESC_F_NEXT == 0 {
                            break;
                        }
                        curr = next;
                    }

                    let status_ptr = unsafe {
                        self.dma_vaddr.add(64 * core::mem::size_of::<VirtIOBlkReq>() + pos)
                    };
                    let status = unsafe { *status_ptr };

                    let result = if status == VIRTIO_BLK_S_OK { 0 } else { -1 };

                    if let Some(server) = self.ring_server.as_mut() {
                        let _ = server.complete(user_data, result);
                    }
                }
            }
        }
    }

    pub fn set_ring_server(&mut self, server: IoRingServer) {
        self.ring_server = Some(server);
    }

    pub fn capacity(&self) -> u64 {
        unsafe {
            let ptr = self.transport.config_ptr() as *const u64;
            ptr.read_volatile()
        }
    }

    pub fn block_size(&self) -> u32 {
        4096 // Default to 4KB for Glenda
    }
}
