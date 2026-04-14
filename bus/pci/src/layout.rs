use glenda::cap::{CapPtr, Endpoint};

// Capability Slots
pub const ENDPOINT_SLOT: CapPtr = CapPtr::from(9);
pub const DEVICE_SLOT: CapPtr = CapPtr::from(10);
pub const MMIO_SLOT: CapPtr = CapPtr::from(11);

// Capabilities
pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);

// Virtual Addresses
pub const ECAM_MAP_VA: usize = 0x4000_0000;
pub const REPORT_VA: usize = 0x5000_0000;
