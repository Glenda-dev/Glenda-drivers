use crate::layout::{IRQ_CAP, IRQ_EP, IRQ_EP_SLOT, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::Ns16550a;
use crate::UartService;
use glenda::cap::{Rights, CSPACE_CAP};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::Badge;
use glenda_drivers::interface::DriverService;

impl<'a> DriverService for UartService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");

        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);
        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;
        let irq_badge = Badge::new(1);
        let irq_handler = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;

        // 3. Mint a badged endpoint for IRQ notification
        CSPACE_CAP.mint(self.endpoint.cap(), IRQ_EP_SLOT, irq_badge, Rights::ALL)?;

        log!("Setting notification to {:?}", IRQ_EP);
        // 4. Configure Interrupt
        // We use our badged endpoint to receive interrupts.
        irq_handler.set_notification(IRQ_EP)?;

        // 5. Init Hardware
        // IRQ is enabled by `init_hw`.
        let uart = Ns16550a::new(MMIO_VA, IRQ_CAP);
        uart.init_hw();
        self.uart = Some(uart);
        log!("Driver initialized!");
        Ok(())
    }
    fn enable(&mut self) {}
    fn disable(&mut self) {}
}
