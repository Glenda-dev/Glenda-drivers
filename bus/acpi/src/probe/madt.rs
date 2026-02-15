use glenda::protocol::device::DeviceDescNode;
use alloc::vec::Vec;

pub fn parse(tables: &acpi::AcpiTables<crate::handler::HandlerWrapper>, devices: &mut Vec<DeviceDescNode>) {
    if let Some(madt_mapping) = tables.find_table::<acpi::sdt::madt::Madt>() {
        let madt = madt_mapping.get();
        crate::arch::parse_madt(&*madt, devices);
    }
}
