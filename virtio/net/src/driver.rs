use crate::layout::{DMA_SLOT, DMA_VA, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::net::VirtIONet;
use crate::NetService;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, ResourceService};
use glenda::ipc::Badge;
use glenda_drivers::interface::DriverService;

impl DriverService for NetService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");

        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);

        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;

        let irq = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;
        log!("Got IRQ cap: {:?}", irq);
        irq.set_notification(self.endpoint)?;

        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, DMA_SLOT)?;
        self.res.mmap(Badge::null(), frame, DMA_VA, 4096 * 4)?;

        let mut net = unsafe { VirtIONet::new(MMIO_VA).map_err(|_| Error::Generic)? };

        net.init(DMA_VA as *mut u8, paddr as u64, self.endpoint).map_err(|_| Error::Generic)?;
        glenda::arch::sync::fence();

        self.net = Some(net);

        log!("Initialized! MAC: {:02x?}", self.net.as_ref().unwrap().mac());
        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
