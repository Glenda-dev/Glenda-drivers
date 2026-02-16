use crate::layout::{MAP_VA, MMIO_SLOT};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use glenda::arch::mem::PGSIZE;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::drivers::ProbeDriver;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::Badge;
use virtio_common::consts::*;

pub struct VirtioMmioDriver<'a> {
    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
}

impl<'a> VirtioMmioDriver<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self { dev, res }
    }

    fn identify_device(&self, device_id: u32) -> Option<String> {
        match device_id {
            DEV_ID_NET => Some("virtio-net".to_string()),
            DEV_ID_BLOCK => Some("virtio-block".to_string()),
            DEV_ID_CONSOLE => Some("virtio-console".to_string()),
            DEV_ID_ENTROPY => Some("virtio-rng".to_string()),
            DEV_ID_GPU => Some("virtio-gpu".to_string()),
            DEV_ID_INPUT => Some("virtio-input".to_string()),
            _ => None,
        }
    }
}

impl<'a> ProbeDriver for VirtioMmioDriver<'a> {
    fn probe(&mut self) -> Result<Vec<String>, Error> {
        // 1. Get MMIO for this virtio,mmio device
        // Use a temp UTCB to set the receive window for the capability
        let utcb = unsafe { glenda::ipc::UTCB::new() };
        utcb.set_recv_window(MMIO_SLOT);

        let (frame, paddr, size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got MMIO: paddr={:#x}, size={:#x}", paddr, size);

        // 2. Map it to our address space
        let pages = (size + PGSIZE - 1) / PGSIZE;
        self.res.mmap(Badge::null(), frame, MAP_VA, pages * PGSIZE)?;

        // 3. Read registers
        let mmio_ptr = MAP_VA as *const u32;
        let magic = unsafe { core::ptr::read_volatile(mmio_ptr.add(OFF_MAGIC / 4)) };
        let version = unsafe { core::ptr::read_volatile(mmio_ptr.add(OFF_VERSION / 4)) };
        let device_id = unsafe { core::ptr::read_volatile(mmio_ptr.add(OFF_DEVICE_ID / 4)) };

        log!("VirtIO device: magic={:#x}, version={}, device_id={}", magic, version, device_id);

        if magic != MAGIC_VALUE {
            error!("Invalid magic value!");
            return Err(Error::InvalidArgs);
        }

        // 4. Identify device
        let mut compats = Vec::new();
        if device_id == 0 {
            log!("Placeholder device (ID 0), ignoring.");
        } else if let Some(compat) = self.identify_device(device_id) {
            log!("Identified as {}, updating...", compat);
            compats.push(compat);
        } else {
            error!("Unknown VirtIO device ID: {}", device_id);
            // Don't return error to prevent driver from crashing, just don't update anything.
        }

        // 5. Cleanup mapping
        self.res.munmap(Badge::null(), MAP_VA, pages * PGSIZE)?;
        Ok(compats)
    }
}
