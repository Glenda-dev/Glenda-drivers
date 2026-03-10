use glenda::cap::{CapPtr, Endpoint};

pub const MMIO_VA: usize = 0x7000_0000;
pub const RING_VA: usize = 0x7200_0000;
pub const DMA_VA: usize = 0x7100_0000;

pub const DEVICE_SLOT: CapPtr = CapPtr::from(0x10);
pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);

pub const MMIO_SLOT: CapPtr = CapPtr::from(0x11);
pub const IRQ_SLOT: CapPtr = CapPtr::from(0x12);
pub const RING_SLOT: CapPtr = CapPtr::from(0x15);
pub const DMA_SLOT: CapPtr = CapPtr::from(0x13);
pub const IRQ_NOTIFY_SLOT: CapPtr = CapPtr::from(0x14);
pub const IRQ_NOTIFY_CAP: Endpoint = Endpoint::from(IRQ_NOTIFY_SLOT);

pub const IRQ_BADGE: usize = 0x100;
