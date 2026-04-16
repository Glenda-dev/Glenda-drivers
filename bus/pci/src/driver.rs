use alloc::string::ToString;
use alloc::vec::Vec;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{DeviceService, VSpaceService};
use glenda::ipc::Badge;
use glenda::mem::Perms;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, DeviceNodeMeta, MMIORegion};
use glenda::utils::align::align_up;
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

use crate::layout::{ECAM_MAP_VA, MMIO_SLOT, REPORT_VA};
use crate::platform::PciPlatformOps;

pub struct PciBusDriver<'a> {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    dev: &'a mut DeviceClient,
    res: &'a mut ResourceClient,
    vspace_mgr: &'a mut VSpaceManager,
    cspace_mgr: &'a mut CSpaceManager,

    ecam_base: usize,
    ecam_size: usize,
    ecam_phys: usize,
    mapped_ecam_window: usize,
    bus_count: usize,
    platform: PciPlatformOps,
}

impl<'a> PciBusDriver<'a> {
    pub fn new(
        endpoint: Endpoint,
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        vspace_mgr: &'a mut VSpaceManager,
        cspace_mgr: &'a mut CSpaceManager,
    ) -> Self {
        Self {
            endpoint,
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
            vspace_mgr,
            cspace_mgr,
            ecam_base: 0,
            ecam_size: 0,
            ecam_phys: 0,
            mapped_ecam_window: 0,
            bus_count: 0,
            platform: PciPlatformOps::generic(),
        }
    }

    fn setup_ecam_mapping(&mut self) -> Result<usize, Error> {
        let (frame, paddr, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        if size < PGSIZE {
            return Err(Error::InvalidArgs);
        }

        self.ecam_phys = paddr;
        self.ecam_size = size;
        self.platform = PciPlatformOps::detect(paddr, size);

        let offset = paddr % PGSIZE;
        // 受当前 MMIO 获取/映射接口限制，这里先映射平台配置允许的 ECAM 窗口，
        // 避免一次性映射整段 ECAM 导致用户态资源压力过大。
        let ecam_window = self.platform.mapped_window_bytes(size);
        let map_len = align_up(ecam_window + offset, PGSIZE);
        let pages = map_len / PGSIZE;

        self.vspace_mgr.map_page(
            glenda::cap::Page::from(frame.into()),
            ECAM_MAP_VA,
            Perms::READ | Perms::WRITE,
            pages,
            self.res,
            self.cspace_mgr,
        )?;

        self.ecam_base = ECAM_MAP_VA + offset;
        self.mapped_ecam_window = ecam_window;
        self.bus_count = core::cmp::max(1, ecam_window >> 20);

        log!(
            "PCI ECAM mapped: platform={}, phys={:#x}, total={:#x}, mapped={:#x}, va={:#x}, scan_buses={} (0..={})",
            self.platform.name,
            self.ecam_phys,
            self.ecam_size,
            ecam_window,
            self.ecam_base,
            self.bus_count,
            self.bus_count.saturating_sub(1)
        );

        Ok(pages)
    }

    fn cfg_off(bus: u8, dev: u8, func: u8, reg: usize) -> usize {
        ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | reg
    }

    fn cfg_addr(&self, bus: u8, dev: u8, func: u8, reg: usize) -> usize {
        self.ecam_base + Self::cfg_off(bus, dev, func, reg)
    }

    fn read_u32(&self, bus: u8, dev: u8, func: u8, reg: usize) -> u32 {
        let addr = self.cfg_addr(bus, dev, func, reg);
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    fn read_u16(&self, bus: u8, dev: u8, func: u8, reg: usize) -> u16 {
        let aligned = reg & !0x3;
        let shift = (reg & 0x2) * 8;
        ((self.read_u32(bus, dev, func, aligned) >> shift) & 0xffff) as u16
    }

    fn read_u8(&self, bus: u8, dev: u8, func: u8, reg: usize) -> u8 {
        let aligned = reg & !0x3;
        let shift = (reg & 0x3) * 8;
        ((self.read_u32(bus, dev, func, aligned) >> shift) & 0xff) as u8
    }

    fn device_present(&self, bus: u8, dev: u8, func: u8) -> bool {
        let vendor = self.read_u16(bus, dev, func, 0x00);
        vendor != 0xffff
    }

    fn is_multifunction(&self, bus: u8, dev: u8) -> bool {
        (self.read_u8(bus, dev, 0, 0x0e) & 0x80) != 0
    }

    fn parse_bars(&self, bus: u8, dev: u8, func: u8, header_type: u8) -> Vec<MMIORegion> {
        let mut out = Vec::new();
        let mut bar = 0usize;
        let max_bars = if (header_type & 0x7f) == 0x01 { 2 } else { 6 };

        while bar < max_bars {
            let off = 0x10 + bar * 4;
            let v = self.read_u32(bus, dev, func, off);
            if v == 0 {
                bar += 1;
                continue;
            }

            // I/O BAR
            if (v & 0x1) != 0 {
                let io_base = (v & 0xffff_fffc) as usize;
                if let Some(io_cpu_base) = self.platform.io_bar_cpu_base() {
                    out.push(MMIORegion { base_addr: io_cpu_base + io_base, size: 0x100 });
                } else {
                    warn!(
                        "PCI {:02x}:{:02x}.{} I/O BAR ignored on platform {} (no io_bar_cpu_base)",
                        bus, dev, func, self.platform.name
                    );
                }
                bar += 1;
                continue;
            }

            let bar_type = (v >> 1) & 0x3;
            if bar_type == 0x2 && bar + 1 < max_bars {
                let hi = self.read_u32(bus, dev, func, off + 4);
                let base = (((hi as u64) << 32) | ((v as u64) & 0xffff_fff0)) as usize;
                if base != 0 {
                    out.push(MMIORegion { base_addr: base, size: 0x1000 });
                }
                bar += 2;
            } else {
                let base = (v & 0xffff_fff0) as usize;
                if base != 0 {
                    out.push(MMIORegion { base_addr: base, size: 0x1000 });
                }
                bar += 1;
            }
        }

        out
    }

    fn build_desc_node(&self, bus: u8, dev: u8, func: u8) -> Option<DeviceDescNode> {
        if !self.device_present(bus, dev, func) {
            return None;
        }

        let vendor_id = self.read_u16(bus, dev, func, 0x00);
        let device_id = self.read_u16(bus, dev, func, 0x02);
        let revision = self.read_u8(bus, dev, func, 0x08);
        let prog_if = self.read_u8(bus, dev, func, 0x09);
        let subclass = self.read_u8(bus, dev, func, 0x0a);
        let class_code = self.read_u8(bus, dev, func, 0x0b);
        let header_type = self.read_u8(bus, dev, func, 0x0e);
        let irq_line = self.read_u8(bus, dev, func, 0x3c);
        let irq_pin = self.read_u8(bus, dev, func, 0x3d);

        let mut compatible = Vec::new();
        compatible.push(alloc::format!("pciid:{:04x}:{:04x}", vendor_id, device_id));
        compatible.push(alloc::format!("pci-vendor:{:04x}", vendor_id));
        compatible.push(alloc::format!("pci-device:{:04x}", device_id));
        compatible.push(alloc::format!("pci-vd:{:04x}:{:04x}", vendor_id, device_id));
        compatible.push("pci-device".to_string());

        let mut mmio = self.parse_bars(bus, dev, func, header_type);

        // QEMU pci-serial 在无固件 BAR 分配场景下，I/O BAR 可能报告为 0x1（base=0）。
        // 这里给出平台已知 I/O window 起始地址回退，避免下游 ns16550a 获取 MMIO 时出现 InvalidArgs。
        if mmio.is_empty() && vendor_id == 0x1b36 && device_id == 0x0002 {
            if let Some(io_cpu_base) = self.platform.io_bar_cpu_base() {
                warn!(
                    "PCI {:02x}:{:02x}.{} has unassigned BARs, applying serial MMIO fallback on {} @ {:#x}",
                    bus,
                    dev,
                    func,
                    self.platform.name,
                    io_cpu_base
                );
                mmio.push(MMIORegion { base_addr: io_cpu_base, size: 0x100 });
            }
        }

        let mut irq = Vec::new();
        let irq_source;
        if irq_pin != 0 && irq_line != 0 && irq_line != 0xff {
            irq.push(irq_line as usize);
            irq_source = "cfg.irq_line";
        } else if let Some(mapped_irq) = self.platform.map_intx_irq(bus, dev, irq_pin) {
            irq.push(mapped_irq);
            irq_source = "platform.intx-map";
        } else {
            irq_source = "none";
        }

        log!(
            "PCI discovered {:02x}:{:02x}.{} vid:did={:04x}:{:04x} class={:02x}:{:02x}:{:02x} bars={} irq_source={} irq={:?}",
            bus,
            dev,
            func,
            vendor_id,
            device_id,
            class_code,
            subclass,
            prog_if,
            mmio.len(),
            irq_source,
            irq
        );

        Some(DeviceDescNode {
            parent: usize::MAX,
            desc: DeviceDesc {
                name: alloc::format!("pci-{:02x}:{:02x}.{}", bus, dev, func),
                compatible,
                mmio,
                irq,
            },
            meta: DeviceNodeMeta {
                bus: Some("pci".to_string()),
                unit_addr: Some(((bus as usize) << 8) | ((dev as usize) << 3) | func as usize),
                tags: alloc::vec!["src:runtime".to_string(), "bus:pci".to_string()],
                properties: alloc::vec![
                    ("pci.bus".to_string(), alloc::format!("{}", bus)),
                    ("pci.device".to_string(), alloc::format!("{}", dev)),
                    ("pci.function".to_string(), alloc::format!("{}", func)),
                    ("pci.vendor".to_string(), alloc::format!("0x{:04x}", vendor_id)),
                    ("pci.device_id".to_string(), alloc::format!("0x{:04x}", device_id)),
                    ("pci.class".to_string(), alloc::format!("0x{:02x}", class_code)),
                    ("pci.subclass".to_string(), alloc::format!("0x{:02x}", subclass)),
                    ("pci.prog_if".to_string(), alloc::format!("0x{:02x}", prog_if)),
                    ("pci.revision".to_string(), alloc::format!("0x{:02x}", revision)),
                    ("pci.header_type".to_string(), alloc::format!("0x{:02x}", header_type & 0x7f),),
                    ("pci.ecam_base".to_string(), alloc::format!("{:#x}", self.ecam_phys)),
                    ("pci.platform".to_string(), self.platform.name.to_string(),),
                ],
            },
        })
    }

    pub fn scan(&mut self) -> Result<(), Error> {
        let pages = self.setup_ecam_mapping()?;

        let mut devices = Vec::new();
        let bus_limit = core::cmp::min(self.bus_count, 256);

        for bus_idx in 0..bus_limit {
            let bus = bus_idx as u8;
            for dev in 0u8..32u8 {
                if !self.device_present(bus, dev, 0) {
                    continue;
                }

                if let Some(node) = self.build_desc_node(bus, dev, 0) {
                    devices.push(node);
                }

                if self.is_multifunction(bus, dev) {
                    for func in 1u8..8u8 {
                        if let Some(node) = self.build_desc_node(bus, dev, func) {
                            devices.push(node);
                        }
                    }
                }
            }
        }

        let _ = self.vspace_mgr.unmap(ECAM_MAP_VA, pages);

        log!("PCI enumeration finished, discovered {} functions", devices.len());
        if !devices.is_empty() {
            self.dev.report_via_frame(
                Badge::null(),
                devices,
                self.res,
                self.vspace_mgr,
                self.cspace_mgr,
                REPORT_VA,
            )?;
        }

        Ok(())
    }
}
