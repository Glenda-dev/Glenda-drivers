use glenda::cap::{CapPtr, Endpoint, Mmio};

// The actual capability we will use to listen for requests
pub const DEVICE_SLOT: CapPtr = CapPtr::from(8);
pub const ENDPOINT_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const DTB_FRAME_SLOT: CapPtr = CapPtr::from(11);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: Mmio = Mmio::from(MMIO_SLOT);

// Where we map the device tree / ACPI tables in our VSpace
pub const MAP_VA: usize = 0x4000_0000;
