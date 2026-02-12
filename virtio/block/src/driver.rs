use crate::layout::{DMA_SLOT, DMA_VA, IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::BlockService;
use crate::VirtIOBlk;
use core::ptr::NonNull;
use glenda::arch::mem::PGSIZE;
use glenda::error::Error;
use glenda::interface::drivers::BlockDriver;
use glenda::interface::{DeviceService, DriverService, MemoryService, ResourceService};
use glenda::ipc::{Badge, UTCB};

impl DriverService for BlockService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap
        utcb.set_recv_window(MMIO_SLOT);
        let _ = self.dev.get_mmio(Badge::null())?;

        // 2. Map MMIO
        self.res.mmap(Badge::null(), MMIO_CAP, MMIO_VA, 0x1000)?;

        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let _ = self.dev.get_irq(Badge::null())?;
        // 4. Configure Interrupt
        IRQ_CAP.set_notification(self.endpoint)?;

        // 5. Init Hardware
        let mut blk = unsafe {
            VirtIOBlk::new(NonNull::new(MMIO_VA as *mut u8).unwrap())
                .expect("Failed to init virtio-blk")
        };
        blk.init_hardware().expect("Failed to init hardware");

        let cap = blk.capacity();
        log!("Capacity: {} sectors ({} MB)", cap, (cap * 512) / (1024 * 1024));

        // 6. Allocate DMA memory (1 page)
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 1, DMA_SLOT)?;
        self.res.mmap(Badge::null(), frame, DMA_VA, PGSIZE)?;
        blk.set_dma(DMA_VA as *mut u8, paddr as u64);
        blk.init_queue(0).expect("Failed to init queue");

        log!("VirtIO-Blk initialized!");

        // --- TEST READ ---
        let mut test_buf = [0u8; 512];
        match blk.read_blocks(0, &mut test_buf) {
            Ok(_) => {
                log!("Test read sector 0 success!");
                log!("First 16 bytes: {:02x?}", &test_buf[0..16]);
            }
            Err(e) => log!("Test read failed: {:?}", e),
        }

        self.blk = Some(blk);
        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
