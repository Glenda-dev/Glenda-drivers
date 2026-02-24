use crate::layout::{IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::GoldfishRtc;
use crate::RtcService;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::{Badge, UTCB};
use glenda_drivers::interface::DriverService;

impl DriverService for RtcService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap
        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;

        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let irq = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;

        // 4. Configure Interrupt
        irq.set_notification(self.endpoint)?;
        irq.set_priority(1)?;
        irq.ack()?;

        // 5. Init Hardware
        let rtc = GoldfishRtc::new(MMIO_VA);
        let unix_time = rtc.get_time();
        log!("Goldfish RTC initialized at {:#x}", MMIO_VA);
        log!("Current RTC time: {}", unix_time);
        self.rtc = Some(rtc);

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
