use glenda::cap::{CapPtr, Endpoint};

// The actual capability we will use to listen for requests
pub const DEVICE_SLOT: CapPtr = CapPtr::from(11);
pub const ENDPOINT_SLOT: CapPtr = CapPtr::from(12);
pub const MMIO_SLOT: CapPtr = CapPtr::from(13);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);

// Where we map the device tree / ACPI tables in our VSpace
pub const MAP_VA: usize = 0x4000_0000;
