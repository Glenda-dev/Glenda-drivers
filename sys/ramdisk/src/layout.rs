use glenda::cap::CapPtr;

pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const RING_SLOT: CapPtr = CapPtr::from(11);
pub const BUFFER_SLOT: CapPtr = CapPtr::from(12);
pub const NOTIFY_SLOT: CapPtr = CapPtr::from(13);

pub const MMIO_VA: usize = 0x6000_0000;
pub const RING_VA: usize = 0x5000_0000;
pub const BUFFER_VA: usize = 0x4000_0000;
