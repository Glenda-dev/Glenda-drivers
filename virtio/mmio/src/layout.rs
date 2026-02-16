use glenda::cap::{CapPtr, Endpoint, Frame};

pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: Frame = Frame::from(MMIO_SLOT);

pub const MAP_VA: usize = 0x6000_0000;
