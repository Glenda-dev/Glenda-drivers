use crate::layout::{DTB_FRAME_SLOT, MAP_VA, MMIO_CAP, MMIO_SLOT};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::drivers::BusDriver;
use glenda::interface::{DeviceService, DriverService, MemoryService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};
use glenda::utils::align::align_up;

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
        utcb.set_recv_window(MMIO_SLOT);
        let (mmio_cap, fdt_addr, fdt_size) = self.dev.get_mmio(Badge::null(), 0)?;
        log!("Got DTB MMIO: cap={:?}, addr={:#x}, size={:#x}", mmio_cap, fdt_addr, fdt_size);
        let pages = align_up(fdt_size, PGSIZE) / PGSIZE;
        MMIO_CAP.get_frame(fdt_addr, pages, DTB_FRAME_SLOT)?;
        let fdt_cap = Frame::from(DTB_FRAME_SLOT);

        // 2. Map DTB
        self.res.mmap(Badge::null(), fdt_cap, MAP_VA, fdt_size)?;

        // 3. Parse DTB
        let fdt_slice = unsafe { core::slice::from_raw_parts(MAP_VA as *const u8, fdt_size) };
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
