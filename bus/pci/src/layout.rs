use glenda::cap::{CapPtr, Endpoint, Frame, PageTable, Reply};

// Capability Slots
pub const ENDPOINT_SLOT: CapPtr = CapPtr::from(10);
pub const DEVICE_SLOT: CapPtr = CapPtr::from(11);
pub const MMIO_SLOT: CapPtr = CapPtr::from(12);
pub const ECAM_FRAME_SLOT_BASE: CapPtr = CapPtr::from(20);

// Capabilities
pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: glenda::cap::Mmio = glenda::cap::Mmio::from(MMIO_SLOT);

// Virtual Addresses
pub const ECAM_VA_BASE: usize = 0x4000_0000;
pub const ECAM_SIZE: usize = 0x1000_0000; // 256MB for full bus 0-255 coverage
