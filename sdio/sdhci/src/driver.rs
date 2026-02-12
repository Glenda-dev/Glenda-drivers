use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::sdhci::Sdhci;
use crate::SdhciService;
use glenda::error::Error;
use glenda::interface::{DeviceService, DriverService, MemoryService};
use glenda::ipc::{Badge, UTCB};

impl DriverService for SdhciService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("SDHCI Driver init...");
        let utcb = unsafe { UTCB::new() };

        utcb.set_recv_window(MMIO_SLOT);
        let _ = self.dev.get_mmio(Badge::null())?;
        self.res.mmap(Badge::null(), MMIO_CAP, MMIO_VA, 0x1000)?;

        utcb.set_recv_window(IRQ_SLOT);
        let _ = self.dev.get_irq(Badge::null())?;
        IRQ_CAP.set_notification(self.endpoint)?;
        IRQ_CAP.set_priority(1)?;
        IRQ_CAP.ack()?;

        self.sdhci = Some(Sdhci::new(MMIO_VA));
        log!("SDHCI initialized at 0x{:x}", MMIO_VA);

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
