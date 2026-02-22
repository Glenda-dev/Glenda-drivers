use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::GpioService;
use crate::SiFiveGpio;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::{Badge, UTCB};
use glenda_drivers::interface::DriverService;

impl DriverService for GpioService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("SiFive GPIO Driver init...");
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
        IRQ_CAP.set_priority(1)?;
        IRQ_CAP.ack()?;

        // 5. Init Hardware
        self.gpio = Some(SiFiveGpio::new(MMIO_VA));
        log!("SiFive GPIO initialized at {:#x}", MMIO_VA);

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
