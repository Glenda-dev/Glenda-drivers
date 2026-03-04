use crate::layout::{IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::GoldfishRtc;
use crate::RtcService;
use glenda::cap::{Rights, CSPACE_CAP};
use glenda::drivers::interface::DriverService;
use glenda::error::Error;
use glenda::interface::{DeviceService, VSpaceService};
use glenda::ipc::Badge;
use glenda::mem::Perms;
use glenda::protocol::device::{LogicDeviceDesc, LogicDeviceType};

impl DriverService for RtcService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");

        // 1. Get MMIO Cap
        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);

        // 2. Map MMIO
        self.vspace.map_frame(
            mmio,
            MMIO_VA,
            Perms::READ | Perms::WRITE,
            1,
            self.res,
            self.cspace,
        )?;
        // 3. Get IRQ Cap
        let irq_badge = Badge::new(1);
        let irq = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;

        // Mint a badged endpoint for IRQ notification
        CSPACE_CAP.mint(self.endpoint.cap(), self.recv, irq_badge, Rights::ALL)?;

        // 4. Configure Interrupt
        irq.set_notification(self.endpoint)?;
        irq.ack()?;

        // 5. Init Hardware
        let rtc = GoldfishRtc::new(MMIO_VA);
        let unix_time = rtc.get_time();
        log!("Goldfish RTC initialized at {:#x}", MMIO_VA);
        log!("Current RTC time: {}", unix_time);
        self.rtc = Some(rtc);

        // 6. Register Logic Device
        let desc = LogicDeviceDesc {
            name: "goldfish-rtc".into(),
            dev_type: LogicDeviceType::Timer,
            parent_name: "platform".into(),
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, self.endpoint.cap())?;
        log!("Registered as Timer logical device");

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
