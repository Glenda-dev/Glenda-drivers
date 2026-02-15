use crate::layout::{DTB_FRAME_SLOT, MAP_VA};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DeviceService, DriverService, MemoryService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};

pub struct DtbDriver<'a> {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
}

impl<'a> DtbDriver<'a> {
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

    fn parse_node(
        &self,
        node: fdt::node::FdtNode,
        parent_idx: usize,
        devices: &mut Vec<DeviceDescNode>,
    ) {
        let name = node.name.to_string();
        let compatible: Vec<String> =
            node.compatible().map(|c| c.all().map(|s| s.to_string()).collect()).unwrap_or_default();

        let mut mmio_regions = Vec::new();
        let mut irqs = Vec::new();

        if let Some(reg) = node.reg() {
            for r in reg {
                if let Some(size) = r.size {
                    mmio_regions.push(MMIORegion {
                        base_addr: r.starting_address as usize,
                        size: size as usize,
                    });
                }
            }
        }

        // Interrupts parsing logic could be complex depending on controller
        // For now, let's keep it simple or empty as before
        if let Some(interrupts) = node.interrupts() {
            for i in interrupts {
                irqs.push(i as usize);
            }
        }

        let desc = DeviceDesc { name, compatible, mmio: mmio_regions, irq: irqs };

        let current_idx = devices.len();
        devices.push(DeviceDescNode { parent: parent_idx, desc });

        for child in node.children() {
            self.parse_node(child, current_idx, devices);
        }
    }
}

impl<'a> BusDriver for DtbDriver<'a> {
    fn probe(&mut self) -> Result<Vec<DeviceDescNode>, Error> {
        // 1. Get DTB MMIO from Device Manager
        let utcb = unsafe { glenda::ipc::UTCB::new() };
        utcb.set_recv_window(DTB_FRAME_SLOT);
        let (fdt_cap, fdt_addr, fdt_size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got DTB MMIO: cap={:?}, addr={:#x}, size={:#x}", fdt_cap, fdt_addr, fdt_size);

        let size = if fdt_size > 0 { fdt_size } else { 0x10000 };

        // 2. Map DTB
        self.res.mmap(Badge::null(), fdt_cap, MAP_VA, size)?;

        // 3. Parse DTB
        let fdt_slice = unsafe { core::slice::from_raw_parts(MAP_VA as *const u8, size) };
        let fdt = fdt::Fdt::new(fdt_slice).map_err(|_| Error::InvalidArgs)?;

        let mut devices = Vec::new();

        // Start from root, index MAX for root parent
        if let Some(root) = fdt.find_node("/") {
            self.parse_node(root, usize::MAX, &mut devices);
        }

        Ok(devices)
    }
}

impl<'a> DriverService for DtbDriver<'a> {
    fn init(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn enable(&mut self) {}
    fn disable(&mut self) {}
}
