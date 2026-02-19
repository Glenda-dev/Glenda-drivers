use glenda::cap::CapPtr;

pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const RING_SLOT: CapPtr = CapPtr::from(11);

pub const MMIO_VA: usize = 0x6000_0000;
pub const RING_VA: usize = 0x7000_0000;
