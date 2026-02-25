use crate::layout::{IRQ_EP, IRQ_EP_SLOT, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::GpioService;
use crate::SiFiveGpio;
use glenda::cap::{Rights, CSPACE_CAP};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::Badge;
use glenda_drivers::interface::DriverService;

impl DriverService for GpioService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("SiFive GPIO Driver init...");

        // 1. Get MMIO Cap
        let (mmio, _, _) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;

        // 3. Get IRQ Cap
        let irq_badge = Badge::new(1);
        let irq_handler = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;

        // Mint a badged endpoint for IRQ notification
        CSPACE_CAP.mint(self.endpoint.cap(), IRQ_EP_SLOT, irq_badge, Rights::ALL)?;

        // 4. Configure Interrupt
        irq_handler.set_notification(IRQ_EP)?;
        irq_handler.ack()?;

        // 5. Init Hardware
        self.gpio = Some(SiFiveGpio::new(MMIO_VA));
        log!("SiFive GPIO initialized at {:#x}", MMIO_VA);

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
