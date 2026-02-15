use acpi::{AcpiHandler, AcpiTables, PhysicalMapping};
use alloc::string::ToString;
use alloc::vec::Vec;
use core::ptr::NonNull;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DriverService, MemoryService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};
use glenda::utils::bootinfo::{BootInfo, PlatformType};

use crate::layout::{BOOTINFO_FRAME_SLOT, BOOTINFO_VA, DYNAMIC_SLOT_BASE, MAP_VA_BASE, MMIO_CAP};

pub struct AcpiDriver<'a> {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
}

impl<'a> AcpiDriver<'a> {
    pub fn new(endpoint: Endpoint, dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            endpoint,
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }

    pub fn listen(&mut self, ep: Endpoint, reply: Reply, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = reply;
        self.recv = recv;
        Ok(())
    }
}

// Context for the handler
struct DriverContext {
    res: *mut ResourceClient,
    va_allocator: usize,
    slot_allocator: usize,
}

#[derive(Clone)]
struct HandlerWrapper {
    ctx: *mut DriverContext,
}

impl AcpiHandler for HandlerWrapper {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let ctx = &mut *self.ctx;
        let res = &mut *ctx.res;

        let paddr_aligned = physical_address & !(PGSIZE - 1);
        let offset = physical_address - paddr_aligned;
        let size_aligned = (size + offset + PGSIZE - 1) & !(PGSIZE - 1);
        let pages = size_aligned / PGSIZE;

        // Alloc VA
        let va = ctx.va_allocator;
        ctx.va_allocator += size_aligned;

        // Alloc Slot
        let slot = CapPtr::from(ctx.slot_allocator);
        ctx.slot_allocator += 1;

        // Map using MMIO_CAP
        if let Err(_) = MMIO_CAP.get_frame(paddr_aligned, pages, slot) {
            let _ = glenda::println!("Failed to get frame for ACPI mapping");
        }
        let frame = Frame::from(slot);

        if let Err(_) = res.mmap(Badge::null(), frame, va, size_aligned) {
            let _ = glenda::println!("Failed to mmap ACPI region");
        }

        PhysicalMapping::new(
            physical_address,
            NonNull::new((va + offset) as *mut T).unwrap(),
            size,
            size_aligned,
            self.clone(),
        )
    }

    fn unmap_physical_region<T>(region: &PhysicalMapping<Self, T>) {
        let ctx = unsafe { &mut *region.handler().ctx };
        let res = unsafe { &mut *ctx.res };

        let va = region.virtual_start().as_ptr() as usize;
        let va_aligned = va & !(PGSIZE - 1);
        res.munmap(Badge::null(), va_aligned, region.mapped_length()).ok();
    }
}

impl<'a> BusDriver for AcpiDriver<'a> {
    fn probe(&mut self) -> Result<Vec<DeviceDescNode>, Error> {
        // 1. Get BootInfo
        let bootinfo_cap = self.res.get_cap(
            Badge::null(),
            glenda::protocol::resource::ResourceType::Bootinfo,
            0,
            BOOTINFO_FRAME_SLOT,
        )?;
        self.res.mmap(Badge::null(), Frame::from(bootinfo_cap), BOOTINFO_VA, PGSIZE)?;

        let bootinfo = unsafe { &*(BOOTINFO_VA as *const BootInfo) };
        let rsdp_addr = if let PlatformType::ACPI = bootinfo.platform_type {
            bootinfo.addr
        } else {
            return Err(Error::NotFound);
        };

        let mut ctx = DriverContext {
            res: self.res as *mut _,
            va_allocator: MAP_VA_BASE,
            slot_allocator: DYNAMIC_SLOT_BASE,
        };

        let handler = HandlerWrapper { ctx: &mut ctx as *mut _ };

        // Use unsafe block for creating tables as required by `acpi` crate
        let tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr) };

        let mut devices = Vec::new();

        if let Ok(tables) = tables {
            // Extract MCFG (PCI)
            // `find_table` returns `Result<PhysicalMapping<H, T>, AcpiError>`
            // We need to handle potential errors or generic table access if `find_table` fails to type check
            // assuming `acpi` crate has `Mcfg` struct.
            if let Ok(mcfg) = tables.find_table::<acpi::mcfg::Mcfg>() {
                for entry in mcfg.entries() {
                    let desc = DeviceDesc {
                        name: "pci-host-ecam".to_string(),
                        compatible: alloc::vec![
                            "pci-host-ecam".to_string(),
                            "pci,ecam".to_string()
                        ],
                        mmio: alloc::vec![MMIORegion {
                            base_addr: entry.base_address as usize,
                            size: 0 // Size 0 implies we don't know or full range
                        }],
                        irq: Vec::new(),
                    };

                    devices.push(DeviceDescNode { parent: usize::MAX, desc });
                }
            }
        }

        Ok(devices)
    }
}

impl<'a> DriverService for AcpiDriver<'a> {
    fn init(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn enable(&mut self) {}
    fn disable(&mut self) {}
}
