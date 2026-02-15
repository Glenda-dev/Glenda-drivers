use crate::handler::{DriverContext, HandlerWrapper};
use crate::layout::{BOOTINFO_FRAME_SLOT, DYNAMIC_SLOT_BASE, MAP_VA_BASE};
use acpi::AcpiTables;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DeviceService, DriverService};
use glenda::ipc::Badge;
use glenda::protocol::device::DeviceDescNode;

pub struct AcpiDriver<'a> {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
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

impl<'a> BusDriver for AcpiDriver<'a> {
    fn probe(&mut self) -> Result<Vec<DeviceDescNode>, Error> {
        log!("Requesting RSDP address...");
        // Get RSDP address via get_mmio
        let utcb = unsafe { glenda::ipc::UTCB::new() };
        utcb.set_recv_window(BOOTINFO_FRAME_SLOT); // Use temporary slot
        let (_, rsdp_addr, _) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("RSDP Address: {:#x}", rsdp_addr);

        let mut ctx = DriverContext {
            res: self.res as *mut _,
            va_allocator: MAP_VA_BASE,
            slot_allocator: DYNAMIC_SLOT_BASE,
        };

        let handler = HandlerWrapper { ctx: &mut ctx as *mut _ };

        // Use unsafe block for creating tables as required by `acpi` crate
        log!("Parsing ACPI Tables...");
        let tables = unsafe { AcpiTables::from_rsdp(handler.clone(), rsdp_addr) };
        let mut devices = Vec::new();

        match tables {
            Ok(tables) => {
                log!("ACPI Tables parsed successfully.");

                // Use the split probe modules
                crate::probe::madt::parse(&tables, &mut devices);
                crate::probe::pci::parse(&tables, &mut devices);
                crate::probe::hpet::parse(&tables, &mut devices);
                crate::probe::aml::parse(&tables, handler.clone(), &mut devices);

                // FADT
                if let Some(_fadt) = tables.find_table::<acpi::sdt::fadt::Fadt>() {
                    log!("Found FADT Table");
                }
            }
            Err(e) => {
                error!("Failed to parse ACPI tables: {:?}", e);
                return Err(Error::InvalidArgs);
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
