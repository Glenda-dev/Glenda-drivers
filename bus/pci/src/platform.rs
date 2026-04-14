#[derive(Clone, Copy)]
pub struct PciPlatformOps {
    pub name: &'static str,
    io_bar_cpu_base: Option<usize>,
    max_scan_window_bytes: usize,
    intx_routing: IntxRouting,
}

#[derive(Clone, Copy)]
enum IntxRouting {
    None,
    QemuPlicSwizzle { expected_ecam_phys: usize, expected_bus: u8, base_irq: usize },
}

impl PciPlatformOps {
    pub const fn generic() -> Self {
        Self {
            name: "generic-pci-host",
            io_bar_cpu_base: None,
            max_scan_window_bytes: 1 << 20,
            intx_routing: IntxRouting::None,
        }
    }

    pub fn detect(ecam_phys: usize, _ecam_size: usize) -> Self {
        match ecam_phys {
            // QEMU virt: PCI host at 0x3000_0000, INTx routed to PLIC 32..35
            0x3000_0000 => Self {
                name: "qemu-virt-pcie",
                io_bar_cpu_base: Some(0x0300_0000),
                max_scan_window_bytes: 1 << 20,
                intx_routing: IntxRouting::QemuPlicSwizzle {
                    expected_ecam_phys: 0x3000_0000,
                    expected_bus: 0,
                    base_irq: 32,
                },
            },
            // Loongson LS2K host bridge window from DT pcie@1a000000 ranges
            0x1a00_0000 => Self {
                name: "loongson-ls2k-pcie",
                io_bar_cpu_base: Some(0x1800_8000),
                max_scan_window_bytes: 1 << 20,
                intx_routing: IntxRouting::None,
            },
            _ => Self::generic(),
        }
    }

    pub fn mapped_window_bytes(&self, ecam_size: usize) -> usize {
        core::cmp::min(ecam_size, self.max_scan_window_bytes)
    }

    pub fn io_bar_cpu_base(&self) -> Option<usize> {
        self.io_bar_cpu_base
    }

    pub fn map_intx_irq(&self, bus: u8, dev: u8, irq_pin: u8) -> Option<usize> {
        if !(1..=4).contains(&irq_pin) {
            return None;
        }

        match self.intx_routing {
            IntxRouting::None => None,
            IntxRouting::QemuPlicSwizzle {
                expected_ecam_phys,
                expected_bus,
                base_irq,
            } => {
                if bus != expected_bus {
                    return None;
                }
                let swizzled = ((dev as usize) + (irq_pin as usize) - 1) & 0x3;
                let _ = expected_ecam_phys;
                Some(base_irq + swizzled)
            }
        }
    }
}
