use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::Ns16550a;
use crate::UartService;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::{Badge, UTCB};
use glenda_drivers::interface::DriverService;

impl<'a> DriverService for UartService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap
        utcb.set_recv_window(MMIO_SLOT);
        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);
        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;
        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let irq_handler = self.dev.get_irq(Badge::null(), 0)?;
        log!("Setting notification to {:?}", self.endpoint);
        // 4. Configure Interrupt
        // We use our endpoint to receive interrupts.
        // Note: Ideally we should use a badged endpoint to distinguish IRQ from IPC.
        // But for now we assume direct notification.
        irq_handler.set_notification(self.endpoint)?;
        irq_handler.set_priority(1)?;

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
