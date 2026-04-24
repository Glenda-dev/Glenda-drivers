use glenda::cap::{CapPtr, Endpoint, IrqHandler, Page};

pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const IRQ_SLOT: CapPtr = CapPtr::from(11);
pub const IRQ_EP_SLOT: CapPtr = CapPtr::from(12);
pub const RING_SLOT: CapPtr = CapPtr::from(13);
pub const SHM_SLOT: CapPtr = CapPtr::from(14);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: Page = Page::from(MMIO_SLOT);
pub const IRQ_CAP: IrqHandler = IrqHandler::from(IRQ_SLOT);
pub const IRQ_EP: Endpoint = Endpoint::from(IRQ_EP_SLOT);
pub const RING_CAP: Page = Page::from(RING_SLOT);
pub const SHM_CAP: Page = Page::from(SHM_SLOT);
pub const IRQ_BADGE: usize = 1 << 1;

pub const MMIO_VA: usize = 0x4000_0000;
pub const RING_VA: usize = 0x5000_0000;
pub const SHM_VA: usize = 0x6000_0000;
