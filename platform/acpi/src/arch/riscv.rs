use acpi::sdt::madt::Madt;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::mem;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, MMIORegion};

// RISC-V ACPI MADT Entry Types
const MADT_TYPE_RINTC: u8 = 24;
const MADT_TYPE_IMSIC: u8 = 25;
const MADT_TYPE_APLIC: u8 = 26;
const MADT_TYPE_PLIC: u8 = 27;

#[repr(C, packed)]
struct RintcEntry {
    header: acpi::sdt::madt::EntryHeader,
    version: u8,
    reserved: u8,
    flags: u32,
    hart_id: u64,
    id: u32,
    ext_intc_id: u32,
    imsic_base_addr: u64,
    imsic_size: u32,
}

#[repr(C, packed)]
struct AplicEntry {
    header: acpi::sdt::madt::EntryHeader,
    version: u8,
    id: u8,
    flags: u32,
    hardware_id: u64,
    idc_count: u16,
    ext_vector_count: u16,
    gsi_base: u32,
    base_addr: u64,
    size: u32,
}

#[repr(C, packed)]
struct PlicEntry {
    header: acpi::sdt::madt::EntryHeader, // 0..2
    version: u8,                          // 2..3
    id: u8,                               // 3..4
    hardware_id: u64,                     // 4..12
    total_irq: u16,                       // 12..14
    base_addr: u64,                       // 14..22
    size: u32,                            // 22..26
    gsi_base: u32,                        // 26..30
    reserved: [u8; 6],                    // 30..36
}

pub fn parse_madt(madt: &Madt, devices: &mut Vec<DeviceDescNode>) {
    log!("Parsing RISC-V MADT (acpi 6.1.0 Manual)...");

    let madt_ptr = madt as *const Madt as *const u8;
    let mut offset = mem::size_of::<Madt>();
    let length = madt.header.length as usize;

    while offset < length {
        unsafe {
            let entry_header = *(madt_ptr.add(offset) as *const acpi::sdt::madt::EntryHeader);
            let entry_ptr = madt_ptr.add(offset);

            match entry_header.entry_type {
                MADT_TYPE_RINTC => {
                    let rintc = &*(entry_ptr as *const RintcEntry);
                    let hart_id = rintc.hart_id;
                    let id = rintc.id;
                    log!("  Found RINTC: hart_id={}, id={}", hart_id, id);
                }
                MADT_TYPE_APLIC => {
                    let aplic = &*(entry_ptr as *const AplicEntry);
                    let base_addr = aplic.base_addr;
                    let size = aplic.size;
                    log!("  Found APLIC: base={:#x}", base_addr);
                    devices.push(DeviceDescNode {
                        parent: usize::MAX,
                        desc: DeviceDesc {
                            name: format!("aplic@{:#x}", base_addr),
                            compatible: alloc::vec!["riscv,aplic".to_string()],
                            mmio: alloc::vec![MMIORegion {
                                base_addr: base_addr as usize,
                                size: size as usize
                            }],
                            irq: Vec::new(),
                        },
                    });
                }
                MADT_TYPE_PLIC => {
                    let mut hex = alloc::string::String::new();
                    use core::fmt::Write;
                    for i in 0..36 {
                        write!(hex, "{:02x} ", *entry_ptr.add(i)).ok();
                    }

                    let plic = &*(entry_ptr as *const PlicEntry);
                    let base_addr = plic.base_addr;
                    let size = plic.size;
                    log!("  Found PLIC: base={:#x}", base_addr);
                    devices.push(DeviceDescNode {
                        parent: usize::MAX,
                        desc: DeviceDesc {
                            name: format!("plic@{:#x}", base_addr),
                            compatible: alloc::vec![
                                "riscv,plic0".to_string(),
                                "sifive,plic-1.0.0".to_string()
                            ],
                            mmio: alloc::vec![MMIORegion {
                                base_addr: base_addr as usize,
                                size: size as usize
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
