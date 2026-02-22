use crate::layout::{DMA_SLOT, DMA_VA, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::BlockService;
use crate::VirtIOBlk;
use alloc::string::String;
use core::ptr::NonNull;
use glenda::arch::mem::PGSIZE;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, ResourceService};
use glenda::ipc::{Badge, UTCB};
use glenda::protocol::device::LogicDeviceDesc;
use glenda::utils::manager::CSpaceService;
use glenda_drivers::interface::DriverService;
use virtio_common::VirtIOTransport;

impl DriverService for BlockService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };
        // 1. Get MMIO Cap
        utcb.set_recv_window(MMIO_SLOT);
        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;
        glenda::arch::sync::fence();

        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let irq_handler = self.dev.get_irq(Badge::null(), 0)?;
        log!("Got IRQ cap: {:?}", irq_handler);

        // 4. Configure Interrupt
        let irq_notify_slot = self.cspace_mgr.alloc(self.res)?;
        self.cspace_mgr.root().mint(
            self.endpoint.cap(),
            irq_notify_slot,
            crate::server::IRQ_BADGE,
            glenda::cap::Rights::ALL,
        )?;
        irq_handler.set_notification(glenda::cap::Endpoint::from(irq_notify_slot))?;
        self.irq = Some(irq_handler);

        // 5. Init Hardware / Construct VirtIOBlk
        let transport = unsafe {
            VirtIOTransport::new(NonNull::new(MMIO_VA as *mut u8).expect("MMIO_VA is null"))
                .expect("Failed to init transport")
        };
        let mut blk = VirtIOBlk::new(transport);

        // 6. Allocate DMA memory (4 pages)
        log!("Allocating 4 pages of DMA memory...");
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, DMA_SLOT)?;
        log!("Mapping DMA: paddr={:#x}, len={:#x}", paddr, 4 * PGSIZE);
        self.res.mmap(Badge::null(), frame, DMA_VA, 4 * PGSIZE)?;
        glenda::arch::sync::fence();

        // 7. Initialize VirtIOBlk
        blk.init(DMA_VA as *mut u8, paddr as u64, self.endpoint)?;
        glenda::arch::sync::fence();

        let cap = blk.capacity();
        log!("Capacity: {} sectors ({} MB)", cap, (cap * 512) / (1024 * 1024));

        self.blk = Some(blk);
        log!("Registering block device with capacity {} sectors", cap);
        // Register as raw block device logic
        let desc = LogicDeviceDesc {
            name: String::from("virtio-blk"),
            parent_name: String::from("root"),
            dev_type: glenda::protocol::device::LogicDeviceType::RawBlock(cap * 512),
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, self.endpoint.cap())?;

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
