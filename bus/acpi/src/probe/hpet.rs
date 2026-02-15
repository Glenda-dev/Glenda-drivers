use alloc::string::ToString;
use alloc::vec::Vec;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};

pub fn parse(
    tables: &acpi::AcpiTables<crate::handler::HandlerWrapper>,
    devices: &mut Vec<DeviceDescNode>,
) {
    if let Ok(hpet) = acpi::sdt::hpet::HpetInfo::new(tables) {
        log!("Found HPET Table");
        let addr = hpet.base_address;
        log!("  HPET: addr={:#x}", addr);
        devices.push(DeviceDescNode {
            parent: usize::MAX,
            desc: DeviceDesc {
                name: "hpet".to_string(),
                compatible: alloc::vec!["intel,hpet".to_string()],
                mmio: alloc::vec![MMIORegion { base_addr: addr, size: 0x1000 }],
                irq: Vec::new(),
            },
        });
    }
}
