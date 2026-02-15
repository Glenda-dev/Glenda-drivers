use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};

pub fn parse_madt(madt: &acpi::sdt::madt::Madt, devices: &mut Vec<DeviceDescNode>) {
    log!("Found MADT (x86 Manual Parsing)");
    let lapic_addr = madt.local_apic_address;
    log!("  Local APIC Addr: {:#x}", lapic_addr);

    devices.push(DeviceDescNode {
        parent: usize::MAX,
        desc: DeviceDesc {
            name: "lapic".to_string(),
            compatible: alloc::vec!["intel,lapic".to_string()],
            mmio: alloc::vec![MMIORegion { base_addr: lapic_addr as usize, size: 0x1000 }],
            irq: Vec::new(),
        },
    });

    let madt_ptr = madt as *const acpi::sdt::madt::Madt as *const u8;
    let mut offset = core::mem::size_of::<acpi::sdt::madt::Madt>();
    let length = madt.header.length as usize;

    while offset < length {
        unsafe {
            let entry_header = *(madt_ptr.add(offset) as *const acpi::sdt::madt::EntryHeader);
            match entry_header.entry_type {
                0 => {
                    // Local APIC
                    let entry = &*(madt_ptr.add(offset) as *const acpi::sdt::madt::LocalApicEntry);
                    if entry.flags & 1 != 0 {
                        log!("  Found CPU: id={}, apic_id={}", entry.processor_id, entry.apic_id);
                    }
                }
                1 => {
                    // IO APIC
                    let ioapic = &*(madt_ptr.add(offset) as *const acpi::sdt::madt::IoApicEntry);
                    let id = ioapic.io_apic_id;
                    let addr = ioapic.io_apic_address;
                    let gsi = ioapic.global_system_interrupt_base;
                    log!("  Found IOAPIC: id={}, addr={:#x}, gsi={}", id, addr, gsi);
                    devices.push(DeviceDescNode {
                        parent: usize::MAX,
                        desc: DeviceDesc {
                            name: format!("ioapic@{:x}", addr),
                            compatible: alloc::vec!["intel,ioapic".to_string()],
                            mmio: alloc::vec![MMIORegion {
                                base_addr: addr as usize,
                                size: 0x1000
                            }],
                            irq: Vec::new(),
                        },
                    });
                }
                _ => {}
            }
            offset += entry_header.length as usize;
        }
    }
}
