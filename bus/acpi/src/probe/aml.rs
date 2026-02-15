use acpi::Handler;
use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use aml::{AmlContext, AmlName, AmlValue, DebugVerbosity};
use glenda::protocol::device::{DeviceDesc, DeviceDescNode};

pub fn parse(
    tables: &acpi::AcpiTables<crate::handler::HandlerWrapper>,
    handler: crate::handler::HandlerWrapper,
    devices: &mut Vec<DeviceDescNode>,
) {
    let mut aml_context = AmlContext::new(Box::new(handler.clone()), DebugVerbosity::None);

    if let Ok(dsdt) = tables.dsdt() {
        log!("AML: Parsing DSDT (addr={:#x}, len={:#x})", dsdt.phys_address, dsdt.length);
        let mapping =
            unsafe { handler.map_physical_region::<u8>(dsdt.phys_address, dsdt.length as usize) };
        let table_bytes = unsafe {
            core::slice::from_raw_parts(mapping.virtual_start.as_ptr(), mapping.region_length)
        };
        // Debug: log first 16 bytes
        let mut hex = alloc::string::String::new();
        use core::fmt::Write;
        for j in 0..16 {
            if j < table_bytes.len() {
                write!(hex, "{:02X} ", table_bytes[j]).ok();
            }
        }
        log!("AML: DSDT Header prefix: {}", hex);
        if let Err(e) = aml_context.parse_table(table_bytes) {
            error!("AML: Failed to parse DSDT: {:?}", e);
        }
    }

    for ssdt in tables.ssdts() {
        log!("AML: Parsing SSDT (addr={:#x}, len={:#x})", ssdt.phys_address, ssdt.length);
        let mapping =
            unsafe { handler.map_physical_region::<u8>(ssdt.phys_address, ssdt.length as usize) };
        let table_bytes = unsafe {
            core::slice::from_raw_parts(mapping.virtual_start.as_ptr(), mapping.region_length)
        };
        if let Err(e) = aml_context.parse_table(table_bytes) {
            error!("AML: Failed to parse SSDT: {:?}", e);
        }
    }

    if let Err(e) = aml_context.initialize_objects() {
        error!("AML: Failed to initialize objects: {:?}", e);
    }

    log!("AML: Initialization complete. Namespace traversal follows.");

    let mut paths = Vec::new();
    let _ = aml_context.namespace.traverse(|name, _level| {
        if *name != AmlName::root() {
            paths.push(name.clone());
        }
        Ok(true)
    });

    for path in paths {
        let value = aml_context.namespace.get_by_path(&path);
        if let Ok(AmlValue::Device) = value {
            let path_str = path.as_string();
            // Try to get _HID. AmlName::from_str handles dots correctly.
            let hid_path = match AmlName::from_str(&(path_str.clone() + "._HID")) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let hid_val = match aml_context.namespace.get_by_path(&hid_path) {
                Ok(AmlValue::Method { .. }) => {
                    aml_context.invoke_method(&hid_path, aml::value::Args::EMPTY).ok()
                }
                Ok(v) => Some(v.clone()),
                Err(_) => None,
            };

            if let Some(val) = hid_val {
                let hid = match val {
                    AmlValue::String(s) => s,
                    AmlValue::Integer(i) => {
                        // Convert EISA ID
                        let mut s = alloc::string::String::new();
                        let cid = (i & 0xFFFFFFFF) as u32;
                        s.push((((cid >> 10) & 0x1f) as u8 + 0x40) as char);
                        s.push((((cid >> 5) & 0x1f) as u8 + 0x40) as char);
                        s.push(((cid & 0x1f) as u8 + 0x40) as char);
                        use core::fmt::Write;
                        write!(s, "{:04X}", (cid >> 16) as u16).ok();
                        s
                    }
                    _ => continue,
                };
                log!("AML: Found device {} with _HID: {}", path_str, hid);

                // Try to get _CRS (Current Resource Settings)
                let crs_path = match AmlName::from_str(&(path_str.clone() + "._CRS")) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let crs_val = match aml_context.namespace.get_by_path(&crs_path) {
                    Ok(AmlValue::Method { .. }) => {
                        aml_context.invoke_method(&crs_path, aml::value::Args::EMPTY).ok()
                    }
                    Ok(v) => Some(v.clone()),
                    Err(_) => None,
                };

                let mut mmio = Vec::new();
                let mut irq = Vec::new();

                if let Some(AmlValue::Buffer(arc_buffer)) = crs_val {
                    let buffer = arc_buffer.lock();
                    let mut i = 0;
                    while i < buffer.len() {
                        let tag = buffer[i];
                        if tag & 0x80 != 0 {
                            // Large descriptor
                            if i + 3 > buffer.len() {
                                break;
                            }
                            let len = u16::from_le_bytes([buffer[i + 1], buffer[i + 2]]) as usize;
                            if i + 3 + len > buffer.len() {
                                break;
                            }
                            let type_id = tag & 0x7f;
                            let data = &buffer[i + 3..i + 3 + len];

                            if type_id == 6 {
                                // Memory32Fixed
                                let base = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                                let size = u32::from_le_bytes([data[5], data[6], data[7], data[8]]);
                                mmio.push(glenda::protocol::device::MMIORegion {
                                    base_addr: base as usize,
                                    size: size as usize,
                                });
                            }
                            i += 3 + len;
                        } else {
                            // Small descriptor
                            let len = (tag & 0x07) as usize;
                            let type_id = (tag >> 3) & 0x0f;
                            if type_id == 4 {
                                // IRQ
                                if len >= 2 {
                                    let mask = u16::from_le_bytes([buffer[i + 1], buffer[i + 2]]);
                                    for bit in 0..16 {
                                        if mask & (1 << bit) != 0 {
                                            irq.push(bit as usize);
                                        }
                                    }
                                }
                            }
                            i += 1 + len;
                            if tag & 0x78 == 0x78 {
                                break;
                            } // End Tag
                        }
                    }
                }

                // Add to discovered devices
                devices.push(DeviceDescNode {
                    parent: usize::MAX,
                    desc: DeviceDesc {
                        name: path_str.split('.').last().unwrap_or("unknown").to_string(),
                        compatible: vec![hid],
                        mmio,
                        irq,
                    },
                });
            }
        }
    }
}
