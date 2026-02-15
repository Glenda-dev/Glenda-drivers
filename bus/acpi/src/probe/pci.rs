use alloc::string::ToString;
use alloc::vec::Vec;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};

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
            log!("  ECAM: addr={:#x}, segment={}", base, seg);
            devices.push(DeviceDescNode {
                parent: usize::MAX,
                desc: DeviceDesc {
                    name: "pci-host-ecam".to_string(),
                    compatible: alloc::vec!["pci-host-ecam".to_string(), "pci,ecam".to_string()],
                    mmio: alloc::vec![MMIORegion { base_addr: base as usize, size: 0 }],
                    irq: Vec::new(),
                },
            });
        }
    }
}
