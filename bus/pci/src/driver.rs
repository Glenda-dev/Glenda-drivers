use alloc::vec::Vec;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode};

use crate::layout::{ECAM_FRAME_SLOT_BASE, ECAM_SIZE, ECAM_VA_BASE, MMIO_CAP};
use crate::pci::PciConfig;

pub struct PciBusDriver<'a> {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,

    ecam_base: usize,
    ecam_size: usize,
}

impl<'a> PciBusDriver<'a> {
    pub fn new(endpoint: Endpoint, dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            endpoint,
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
            ecam_base: 0,
            ecam_size: 0,
        }
    }

    pub fn scan(&mut self) -> Result<(), Error> {
        // ... (same as before) ...

        let ecam_phys = 0x3000_0000;

        self.ecam_base = ECAM_VA_BASE;
        self.ecam_size = ECAM_SIZE;

        // Map ECAM
        let pages = self.ecam_size / PGSIZE;
        MMIO_CAP.get_frame(ecam_phys, pages, ECAM_FRAME_SLOT_BASE)?;
        self.res.mmap(
            Badge::null(),
            Frame::from(ECAM_FRAME_SLOT_BASE),
            self.ecam_base,
            self.ecam_size,
        )?;

        // Enumerate
        let mut devices = Vec::new();
        for bus in 0..=u8::MAX {
            for dev in 0..32 {
                for func in 0..8 {
                    if let Some(desc) = self.check_device(bus, dev, func) {
                        devices.push(desc);
                    }
                }
            }
        }

        // Report
        // Need to import DeviceService trait to use .report()
        if !devices.is_empty() {
            self.dev.report(Badge::null(), devices)?;
        }

        Ok(())
    }

    fn check_device(&self, bus: u8, dev: u8, func: u8) -> Option<DeviceDescNode> {
        let offset = ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12);
        if offset >= self.ecam_size {
            return None;
        }

        let addr = self.ecam_base + offset;
        // Safety: We mapped ECAM region.
        let config = unsafe { &*(addr as *const PciConfig) };

        // Copy to local variables to avoid unaligned access
        let vendor_id = config.vendor_id;
        let device_id = config.device_id;

        if vendor_id == 0xFFFF {
            return None;
        }

        // Found a device
        let mut compatible = Vec::new();
        compatible.push(alloc::format!("pci{:04x},{:04x}", vendor_id, device_id));
        compatible.push("pci-device".into());

        // Determine class/subclass to add more specific compatible strings?
        // e.g., virtio
        if vendor_id == 0x1AF4 {
            if device_id >= 0x1000 && device_id <= 0x107F {
                compatible.push("virtio-pci".into());
            }
        }

        let desc = DeviceDesc {
            name: alloc::format!("pci-{:02x}:{:02x}.{}", bus, dev, func),
            compatible,
            mmio: Vec::new(), // BARs should be parsed here ideally
            irq: Vec::new(),  // Interrupt line/pin
        };

        Some(DeviceDescNode {
            parent: usize::MAX, // Attached to PCI Root (us)
            desc,
        })
    }
}
