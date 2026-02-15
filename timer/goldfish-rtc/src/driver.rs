use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::GoldfishRtc;
use crate::RtcService;
use glenda::error::Error;
use glenda::interface::{DeviceService, DriverService, MemoryService};
use glenda::ipc::{Badge, UTCB};

impl DriverService for RtcService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap
        utcb.set_recv_window(MMIO_SLOT);
        let _ = self.dev.get_mmio(Badge::null(), 0)?;

        // 2. Map MMIO
        self.res.mmap(Badge::null(), MMIO_CAP, MMIO_VA, 0x1000)?;

        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let _ = self.dev.get_irq(Badge::null(), 0)?;

        // 4. Configure Interrupt
        IRQ_CAP.set_notification(self.endpoint)?;
        IRQ_CAP.set_priority(1)?;
        IRQ_CAP.ack()?;

        // 5. Init Hardware
        let rtc = GoldfishRtc::new(MMIO_VA);
        let unix_time = rtc.get_time();
        log!("Goldfish RTC initialized at 0x{:x}", MMIO_VA);
        log!("Current RTC time: {}", unix_time);
        self.rtc = Some(rtc);

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
