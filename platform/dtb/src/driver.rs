pub use crate::layout::{DTB_FRAME_SLOT, MAP_VA, MMIO_CAP, MMIO_SLOT};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};
use glenda::utils::align::align_up;
use glenda_drivers::interface::DriverService;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerMethod {
    Sbi,
    Psci,
    Gpio,
    Syscon,
    None,
}

pub struct DtbDriver {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,
    pub thermal_zones: glenda::protocol::device::thermal::ThermalZones,
    pub thermal_base: Option<usize>,

    pub has_power_off: bool,
    pub has_reboot: bool,
    pub power_method: PowerMethod,

    pub dev_client: DeviceClient,
    pub res_client: ResourceClient,
}

impl DtbDriver {
    pub fn new(endpoint: Endpoint, dev_client: DeviceClient, res_client: ResourceClient) -> Self {
        Self {
            endpoint,
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            thermal_zones: glenda::protocol::device::thermal::ThermalZones::default(),
            thermal_base: None,

            has_power_off: false,
            has_reboot: false,
            power_method: PowerMethod::None,

            dev_client,
            res_client,
        }
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

impl DtbDriver {
    pub fn probe(&mut self) -> Result<Vec<DeviceDescNode>, Error> {
        // Assume FDT is mapped at MAP_VA
        let fdt_slice = unsafe { core::slice::from_raw_parts(MAP_VA as *const u8, 0x100000) };
        let fdt = fdt::Fdt::new(fdt_slice).map_err(|_| Error::InvalidArgs)?;

        let mut devices = Vec::new();

        // Start from root, index MAX for root parent
        if let Some(root) = fdt.find_node("/") {
            self.parse_node(root, usize::MAX, &mut devices);
        }

        Ok(devices)
    }
}

impl DriverService for DtbDriver {
    fn init(&mut self) -> Result<(), Error> {
        // 1. Get DTB MMIO from Device Manager
        let utcb = unsafe { glenda::ipc::UTCB::new() };
        utcb.set_recv_window(MMIO_SLOT);
        let (mmio_cap, fdt_addr, fdt_size) = self.dev_client.get_mmio(Badge::null(), 0)?;
        log!("Got DTB MMIO: cap={:?}, addr={:#x}, size={:#x}", mmio_cap, fdt_addr, fdt_size);

        let pages = align_up(fdt_size, PGSIZE) / PGSIZE;
        MMIO_CAP.get_frame(fdt_addr, pages, DTB_FRAME_SLOT)?;
        let fdt_cap = Frame::from(DTB_FRAME_SLOT);

        // 2. Map DTB
        self.res_client.mmap(Badge::null(), fdt_cap, MAP_VA, fdt_size)?;

        // 3. Parse DTB for platforms and thermal zones
        let fdt_slice = unsafe { core::slice::from_raw_parts(MAP_VA as *const u8, fdt_size) };
        let fdt = fdt::Fdt::new(fdt_slice).map_err(|_| Error::InvalidArgs)?;

        // Parse Thermal Zones
        if let Some(tz) = fdt.find_node("/thermal-zones") {
            for zone in tz.children() {
                let name = zone.name.to_string();
                let mut trips = Vec::new();
                if let Some(trips_node) = zone.children().find(|c| c.name == "trips") {
                    for trip in trips_node.children() {
                        let temp = trip
                            .property("temperature")
                            .map(|p| p.as_usize().unwrap_or(0) as u32)
                            .unwrap_or(0);
                        let hysteresis = trip
                            .property("hysteresis")
                            .map(|p| p.as_usize().unwrap_or(0) as u32)
                            .unwrap_or(0);
                        let trip_type = match trip.property("type").and_then(|p| p.as_str()) {
                            Some("passive") => glenda::protocol::device::thermal::TripType::Passive,
                            Some("active") => glenda::protocol::device::thermal::TripType::Active,
                            Some("hot") => glenda::protocol::device::thermal::TripType::Hot,
                            Some("critical") => {
                                glenda::protocol::device::thermal::TripType::Critical
                            }
                            _ => glenda::protocol::device::thermal::TripType::Passive,
                        };
                        trips.push(glenda::protocol::device::thermal::ThermalTrip {
                            temp,
                            hysteresis,
                            trip_type,
                        });
                    }
                }

                let t_type = if name.contains("cpu") {
                    glenda::protocol::device::thermal::ThermalType::Cpu
                } else if name.contains("gpu") {
                    glenda::protocol::device::thermal::ThermalType::Gpu
                } else {
                    glenda::protocol::device::thermal::ThermalType::Board
                };

                self.thermal_zones.zones.push(glenda::protocol::device::thermal::ThermalZoneInfo {
                    name,
                    thermal_type: t_type,
                    trips,
                    sensor_id: 0,
                    driver_logic_id: 0,
                });
            }
        }

        // 4. Detect Power/Reboot Capability
        for node in fdt.all_nodes() {
            let compatible = node.compatible().map(|c| c.all());
            if let Some(comp) = compatible {
                for c in comp {
                    match c {
                        "psci" => {
                            log!("Found psci");
                            self.has_power_off = true;
                            self.has_reboot = true;
                            self.power_method = PowerMethod::Psci;
                        }
                        "gpio-poweroff" => {
                            log!("Found gpio-poweroff");
                            self.has_power_off = true;
                            self.power_method = PowerMethod::Gpio;
                        }
                        "gpio-restart" => {
                            log!("Found gpio-restart");
                            self.has_reboot = true;
                            self.power_method = PowerMethod::Gpio;
                        }
                        "syscon-reboot" => {
                            log!("Found syscon-reboot");
                            self.has_reboot = true;
                            self.power_method = PowerMethod::Syscon;
                        }
                        _ => {}
                    }
                }
            }
            if node.name == "sbi" || node.name.starts_with("sbi@") {
                log!("Found sbi node");
                self.has_power_off = true;
                self.has_reboot = true;
                self.power_method = PowerMethod::Sbi;
            }
        }

        Ok(())
    }

    fn enable(&mut self) {
        self.running = true;
    }

    fn disable(&mut self) {
        self.running = false;
    }
}
