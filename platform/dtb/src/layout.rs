use glenda::cap::{CapPtr, Endpoint, Mmio};

pub const BOOTINFO_SLOT: CapPtr = CapPtr::from(2);
pub const RESOURCE_SLOT: CapPtr = CapPtr::from(3);
// The actual capability we will use to listen for requests
pub const DEVICE_SLOT: CapPtr = CapPtr::from(8);
pub const ENDPOINT_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const DTB_FRAME_SLOT: CapPtr = CapPtr::from(11);
pub const BOOTINFO_FRAME_SLOT: CapPtr = CapPtr::from(12);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: Mmio = Mmio::from(MMIO_SLOT);
pub const ENDPOINT_CAP: Endpoint = Endpoint::from(ENDPOINT_SLOT);

// Where we map the device tree / ACPI tables in our VSpace
pub const MAP_VA: usize = 0x4000_0000;
pub const BOOTINFO_VA: usize = 0x5000_0000;
