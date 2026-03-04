use crate::layout::{DMA_SLOT, DMA_VA, IRQ_EP, IRQ_EP_SLOT, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::net::VirtIONet;
use crate::NetService;
use alloc::string::String;
use glenda::cap::{Rights, CSPACE_CAP};
use glenda::error::Error;
use glenda::interface::{DeviceService, ResourceService, VSpaceService};
use glenda::ipc::Badge;
use glenda::protocol::device::LogicDeviceDesc;
use glenda_drivers::interface::DriverService;

impl DriverService for NetService<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");

        let (mmio, pa, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got MMIO cap: addr={:#x}, size={:#x}", pa, size);

        self.vspace_mgr.map_frame(
            mmio,
            MMIO_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace_mgr,
        )?;

        let irq_badge = Badge::new(1);
        let irq = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;
        log!("Got IRQ cap: {:?}", irq);

        // Mint a badged endpoint for IRQ notification
        CSPACE_CAP.mint(self.endpoint.cap(), IRQ_EP_SLOT, irq_badge, Rights::ALL)?;
        irq.set_notification(IRQ_EP)?;

        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, DMA_SLOT)?;
        self.vspace_mgr.map_frame(
            frame,
            DMA_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            4,
            self.res,
            self.cspace_mgr,
        )?;

        let mut net = unsafe { VirtIONet::new(MMIO_VA).map_err(|_| Error::Generic)? };

        net.init(DMA_VA as *mut u8, paddr as u64, self.endpoint).map_err(|_| Error::Generic)?;
        glenda::arch::sync::fence();

        self.net = Some(net);

        let mac = self.net.as_ref().unwrap().mac();
        log!("Initialized! MAC: {:02x?}", mac);

        // Register as logical network device with Unicorn
        let desc = LogicDeviceDesc {
            name: String::from("virtio-net"),
            parent_name: String::from("root"),
            dev_type: glenda::protocol::device::LogicDeviceType::Net,
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, self.endpoint.cap())?;

        Ok(())
    }

    fn enable(&mut self) {}

    fn disable(&mut self) {}
}
