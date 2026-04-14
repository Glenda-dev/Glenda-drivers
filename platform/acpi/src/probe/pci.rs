use alloc::string::ToString;
use alloc::vec::Vec;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, DeviceNodeMeta, MMIORegion};

pub fn parse(
    tables: &acpi::AcpiTables<crate::handler::HandlerWrapper>,
    devices: &mut Vec<DeviceDescNode>,
) {
    if let Some(mcfg_mapping) = tables.find_table::<acpi::sdt::mcfg::Mcfg>() {
        let mcfg = mcfg_mapping.get();
        log!("Found MCFG Table (PCI Express ECAM)");
        for entry in mcfg.entries() {
            let base = entry.base_address;
            let seg = entry.pci_segment_group;
            let bus_start = entry.bus_number_start;
            let bus_end = entry.bus_number_end;
            let bus_count = bus_end.saturating_sub(bus_start) as usize + 1;
            let ecam_size = bus_count << 20;
            log!(
                "  ECAM: addr={:#x}, segment={}, bus-range={}..{}",
                base,
                seg,
                bus_start,
                bus_end
            );
            devices.push(DeviceDescNode {
                parent: usize::MAX,
                desc: DeviceDesc {
                    name: alloc::format!("pci-host-ecam@{:#x}", base),
                    compatible: alloc::vec![
                        "pci-host-ecam".to_string(),
                        "pci,ecam".to_string(),
                        "pci-host-ecam-generic".to_string(),
                    ],
                    mmio: alloc::vec![MMIORegion { base_addr: base as usize, size: ecam_size }],
                    irq: Vec::new(),
                },
                meta: DeviceNodeMeta {
                    bus: Some("pci".to_string()),
                    unit_addr: Some(base as usize),
                    tags: alloc::vec!["src:acpi".to_string(), "acpi:mcfg".to_string()],
                    properties: alloc::vec![
                        ("acpi.mcfg.segment".to_string(), alloc::format!("{}", seg)),
                        ("acpi.mcfg.base".to_string(), alloc::format!("{}", base)),
                        (
                            "acpi.mcfg.bus-range".to_string(),
                            alloc::format!("{}-{}", bus_start, bus_end),
                        ),
                        (
                            "acpi.mcfg.ecam-size".to_string(),
                            alloc::format!("{}", ecam_size),
                        ),
                    ],
                },
            });
        }
    }
}
