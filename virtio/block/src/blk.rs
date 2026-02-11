use core::ptr::NonNull;
use glenda::error::Error;
use glenda::interface::BlockDriver;
use virtio_common::{consts::*, queue::*, VirtIOError, VirtIOTransport};

#[repr(C)]
#[derive(Debug, Default)]
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

pub struct VirtIOBlk {
    transport: VirtIOTransport,
    queue: Option<VirtQueue>,
    // For DMA memory in this simple implementation, we might need pre-allocated buffers.
    // In a real system, we'd use a proper DMA allocator.
    dma_vaddr: *mut u8,
    dma_paddr: u64,
}

impl VirtIOBlk {
    pub unsafe fn new(base: NonNull<u8>) -> Result<Self, VirtIOError> {
        let transport = VirtIOTransport::new(base)?;
        Ok(Self { transport, queue: None, dma_vaddr: core::ptr::null_mut(), dma_paddr: 0 })
    }

    pub fn set_dma(&mut self, vaddr: *mut u8, paddr: u64) {
        self.dma_vaddr = vaddr;
        self.dma_paddr = paddr;
    }

    pub fn init_hardware(&mut self) -> Result<(), VirtIOError> {
        self.transport.set_status(0); // Reset
        self.transport.add_status(STATUS_ACKNOWLEDGE);
        self.transport.add_status(STATUS_DRIVER);

        let features = self.transport.get_features();
        // Just accept what the device offers for now
        self.transport.set_features(features);

        self.transport.add_status(STATUS_FEATURES_OK);
        if self.transport.get_status() & STATUS_FEATURES_OK == 0 {
            return Err(VirtIOError::DeviceNotFound);
        }
        Ok(())
    }

    pub fn init_queue(&mut self, index: u16) -> Result<(), VirtIOError> {
        unsafe {
            self.transport.write_queue_sel(index as u32);
            let max = self.transport.read_queue_max();
            if max == 0 {
                return Err(VirtIOError::InvalidHeader);
            }
            let num = if max > 64 { 64 } else { max as u16 };

            let vq = VirtQueue::new(index as u32, num, self.dma_paddr, self.dma_vaddr);
            self.transport.setup_queue(&vq);
            self.queue = Some(vq);
        }
        self.transport.add_status(STATUS_DRIVER_OK);
        Ok(())
    }
}

impl BlockDriver for VirtIOBlk {
    fn capacity(&self) -> u64 {
        let mut cap: u64 = 0;
        for i in 0..8 {
            cap |= (self.transport.read_config(i) as u64) << (i * 8);
        }
        cap
    }

    fn block_size(&self) -> u32 {
        512
    }

    fn read_blocks(&mut self, sector: u64, buf: &mut [u8]) -> Result<usize, Error> {
        let vq = self.queue.as_mut().ok_or(Error::NotInitialized)?;

        // We need 3 descriptors: Header, Data, Status
        // Use part of DMA memory for these
        // Use 2KB offset into the 4KB page to avoid the VirtQueue which is ~1KB
        let req_vaddr = unsafe { self.dma_vaddr.add(2048) } as *mut VirtIOBlkReq;
        let req_paddr = self.dma_paddr + 2048;

        let status_vaddr = unsafe { self.dma_vaddr.add(2112) } as *mut u8;
        let status_paddr = self.dma_paddr + 2112;

        let data_vaddr = unsafe { self.dma_vaddr.add(2240) };
        let data_paddr = self.dma_paddr + 2240;

        unsafe {
            req_vaddr.write_volatile(VirtIOBlkReq { type_: VIRTIO_BLK_T_IN, reserved: 0, sector });
            status_vaddr.write_volatile(0xff);
        }

        let desc_id_header = vq.alloc_desc().ok_or(Error::OutOfMemory)?;
        let desc_id_data = vq.alloc_desc().ok_or(Error::OutOfMemory)?;
        let desc_id_status = vq.alloc_desc().ok_or(Error::OutOfMemory)?;

        let descs = vq.desc_table();

        // Header
        descs[desc_id_header as usize].addr = req_paddr;
        descs[desc_id_header as usize].len = core::mem::size_of::<VirtIOBlkReq>() as u32;
        descs[desc_id_header as usize].flags = DESC_F_NEXT;
        descs[desc_id_header as usize].next = desc_id_data;

        // Data
        descs[desc_id_data as usize].addr = data_paddr;
        descs[desc_id_data as usize].len = buf.len() as u32;
        descs[desc_id_data as usize].flags = DESC_F_NEXT | DESC_F_WRITE;
        descs[desc_id_data as usize].next = desc_id_status;

        // Status
        descs[desc_id_status as usize].addr = status_paddr;
        descs[desc_id_status as usize].len = 1;
        descs[desc_id_status as usize].flags = DESC_F_WRITE;
        descs[desc_id_status as usize].next = 0;

        vq.submit(desc_id_header);
        glenda::println!("VirtIO-Blk: Notifying queue...");
        self.transport.notify_queue(vq.index);

        // Polling for completion
        glenda::println!("VirtIO-Blk: Waiting for completion...");
        while !vq.can_pop() {
            core::hint::spin_loop();
        }

        glenda::println!("VirtIO-Blk: Popping from queue...");
        vq.pop();

        let status = unsafe { status_vaddr.read_volatile() };

        // Free descriptors
        vq.free_desc(desc_id_status);
        vq.free_desc(desc_id_data);
        vq.free_desc(desc_id_header);

        if status == VIRTIO_BLK_S_OK {
            unsafe {
                core::ptr::copy_nonoverlapping(data_vaddr, buf.as_mut_ptr(), buf.len());
            }
            Ok(buf.len())
        } else {
            Err(Error::IoError)
        }
    }

    fn write_blocks(&mut self, sector: u64, buf: &[u8]) -> Result<usize, Error> {
        let vq = self.queue.as_mut().ok_or(Error::NotInitialized)?;

        let req_vaddr = unsafe { self.dma_vaddr.add(2048) } as *mut VirtIOBlkReq;
        let req_paddr = self.dma_paddr + 2048;

        let status_vaddr = unsafe { self.dma_vaddr.add(2112) } as *mut u8;
        let status_paddr = self.dma_paddr + 2112;

        let data_vaddr = unsafe { self.dma_vaddr.add(2240) };
        let data_paddr = self.dma_paddr + 2240;

        unsafe {
            req_vaddr.write_volatile(VirtIOBlkReq { type_: VIRTIO_BLK_T_OUT, reserved: 0, sector });
            status_vaddr.write_volatile(0xff);
            core::ptr::copy_nonoverlapping(buf.as_ptr(), data_vaddr, buf.len());
        }

        let desc_id_header = vq.alloc_desc().ok_or(Error::OutOfMemory)?;
        let desc_id_data = vq.alloc_desc().ok_or(Error::OutOfMemory)?;
        let desc_id_status = vq.alloc_desc().ok_or(Error::OutOfMemory)?;

        let descs = vq.desc_table();

        // Header
        descs[desc_id_header as usize].addr = req_paddr;
        descs[desc_id_header as usize].len = core::mem::size_of::<VirtIOBlkReq>() as u32;
        descs[desc_id_header as usize].flags = DESC_F_NEXT;
        descs[desc_id_header as usize].next = desc_id_data;

        // Data
        descs[desc_id_data as usize].addr = data_paddr;
        descs[desc_id_data as usize].len = buf.len() as u32;
        descs[desc_id_data as usize].flags = DESC_F_NEXT; // Not write because we output
        descs[desc_id_data as usize].next = desc_id_status;

        // Status
        descs[desc_id_status as usize].addr = status_paddr;
        descs[desc_id_status as usize].len = 1;
        descs[desc_id_status as usize].flags = DESC_F_WRITE;
        descs[desc_id_status as usize].next = 0;

        vq.submit(desc_id_header);
        glenda::println!("VirtIO-Blk: Notifying queue...");
        self.transport.notify_queue(vq.index);

        glenda::println!("VirtIO-Blk: Waiting for completion...");
        while !vq.can_pop() {
            core::hint::spin_loop();
        }
        glenda::println!("VirtIO-Blk: Popping from queue...");

        vq.pop();

        let status = unsafe { status_vaddr.read_volatile() };

        vq.free_desc(desc_id_status);
        vq.free_desc(desc_id_data);
        vq.free_desc(desc_id_header);

        if status == VIRTIO_BLK_S_OK {
            Ok(buf.len())
        } else {
            Err(Error::IoError)
        }
    }

    fn sync(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
