use crate::layout::{DMA_SLOT, DMA_VA, IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::net::VirtIONet;
use crate::NetService;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, ResourceService};
use glenda::ipc::{Badge, UTCB};
use glenda_drivers::interface::DriverService;

impl DriverService for NetService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        utcb.set_recv_window(MMIO_SLOT);
        let _ = self.dev.get_mmio(Badge::null(), 0)?;

        self.res.mmap(Badge::null(), MMIO_CAP, MMIO_VA, 0x1000)?;

        utcb.set_recv_window(IRQ_SLOT);
        let _ = self.dev.get_irq(Badge::null(), 0)?;

        IRQ_CAP.set_notification(self.endpoint)?;

        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, DMA_SLOT)?;
        self.res.mmap(Badge::null(), frame, DMA_VA, 4096 * 4)?;

        let mut net = unsafe { VirtIONet::new(MMIO_VA).map_err(|_| Error::Generic)? };

        net.init(DMA_VA as *mut u8, paddr as u64, self.endpoint).map_err(|_| Error::Generic)?;

        self.net = Some(net);

        log!("Initialized! MAC: {:02x?}", self.net.as_ref().unwrap().mac());
        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
