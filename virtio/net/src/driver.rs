use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_CAP, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::net::VirtIONet;
use crate::NetService;
use glenda::error::Error;
use glenda::interface::{DeviceService, DriverService, MemoryService};
use glenda::ipc::{Badge, UTCB};

impl DriverService for NetService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init ...");
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

        // 5. Init Hardware
        let net = unsafe {
            VirtIONet::new(MMIO_VA).map_err(|_| Error::Unknown).expect("Failed to init virtio-net")
        };
        self.net = Some(net);

        log!("Initialized!");
        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
