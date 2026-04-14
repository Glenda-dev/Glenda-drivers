use alloc::string::{String, ToString};
use alloc::vec::Vec;
use glenda::error::Error;
use glenda::protocol::device::{DeviceDesc, DeviceDescNode, DeviceNodeMeta, MMIORegion};

fn parse_unit_addr(name: &str) -> Option<usize> {
    let (_, suffix) = name.rsplit_once('@')?;
    let suffix = suffix.trim_start_matches("0x");
    usize::from_str_radix(suffix, 16).ok()
}

fn infer_bus(name: &str, compatible: &[String]) -> Option<String> {
    if compatible.iter().any(|c| c.contains("virtio")) {
        return Some("virtio".to_string());
    }
    if compatible.iter().any(|c| c.contains("pci")) {
        return Some("pci".to_string());
    }
    if compatible.iter().any(|c| c.contains("uart") || c.contains("serial")) {
        return Some("serial".to_string());
    }
    if compatible.iter().any(|c| c.contains("simple-bus")) || name == "soc" {
        return Some("platform".to_string());
    }
    None
}

fn collect_meta(node: fdt::node::FdtNode, name: &str, compatible: &[String]) -> DeviceNodeMeta {
    let mut tags = Vec::new();
    tags.push("src:dtb".to_string());

    let mut properties = Vec::new();

    if let Some(status) = node.property("status").and_then(|p| p.as_str()) {
        tags.push(alloc::format!("status:{}", status));
        properties.push(("status".to_string(), status.to_string()));
    }

    if let Some(dev_type) = node.property("device_type").and_then(|p| p.as_str()) {
        tags.push(alloc::format!("type:{}", dev_type));
        properties.push(("device_type".to_string(), dev_type.to_string()));
    }

    if let Some(model) = node.property("model").and_then(|p| p.as_str()) {
        properties.push(("model".to_string(), model.to_string()));
    }

    if let Some(freq) = node.property("clock-frequency").and_then(|p| p.as_usize()) {
        properties.push(("clock-frequency".to_string(), alloc::format!("{}", freq)));
    }

    if let Some(phandle) = node.property("phandle").and_then(|p| p.as_usize()) {
        properties.push(("phandle".to_string(), alloc::format!("{}", phandle)));
    }

    if let Some(addr_cells) = node.property("#address-cells").and_then(|p| p.as_usize()) {
        properties.push(("#address-cells".to_string(), alloc::format!("{}", addr_cells)));
    }

    if let Some(size_cells) = node.property("#size-cells").and_then(|p| p.as_usize()) {
        properties.push(("#size-cells".to_string(), alloc::format!("{}", size_cells)));
    }

    if let Some(pci_domain) = node.property("linux,pci-domain").and_then(|p| p.as_usize()) {
        properties.push(("linux,pci-domain".to_string(), alloc::format!("{}", pci_domain)));
    }

    if let Some(bus_range_prop) = node.property("bus-range") {
        if bus_range_prop.value.len() >= 8 {
            let start = u32::from_be_bytes([
                bus_range_prop.value[0],
                bus_range_prop.value[1],
                bus_range_prop.value[2],
                bus_range_prop.value[3],
            ]);
            let end = u32::from_be_bytes([
                bus_range_prop.value[4],
                bus_range_prop.value[5],
                bus_range_prop.value[6],
                bus_range_prop.value[7],
            ]);
            properties.push(("bus-range".to_string(), alloc::format!("{}-{}", start, end)));
        }
    }

    DeviceNodeMeta {
        bus: infer_bus(name, compatible),
        unit_addr: parse_unit_addr(name),
        tags,
        properties,
    }
}

fn parse_node(node: fdt::node::FdtNode, parent_idx: usize, devices: &mut Vec<DeviceDescNode>) {
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
    let meta = collect_meta(node, &name, &compatible);
    let desc = DeviceDesc { name, compatible, mmio: mmio_regions, irq: irqs };
    let current_idx = devices.len();
    devices.push(DeviceDescNode { parent: parent_idx, desc, meta });

    for child in node.children() {
        parse_node(child, current_idx, devices);
    }
}

pub fn parse_dtb_blob(fdt_slice: &[u8]) -> Result<Vec<DeviceDescNode>, Error> {
    let fdt = fdt::Fdt::new(fdt_slice).map_err(|_| Error::InvalidArgs)?;

    let mut devices = Vec::new();
    if let Some(root) = fdt.find_node("/") {
        parse_node(root, usize::MAX, &mut devices);
    }

    Ok(devices)
}
