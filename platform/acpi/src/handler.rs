use crate::layout::MMIO_CAP;
use acpi::PhysicalMapping;
use core::ptr::NonNull;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CapPtr, Frame};
use glenda::client::ResourceClient;
use glenda::interface::MemoryService;
use glenda::ipc::Badge;

// Context for the handler
pub struct DriverContext {
    pub res: *mut ResourceClient,
    pub va_allocator: usize,
    pub slot_allocator: usize,
}

#[derive(Clone)]
pub struct HandlerWrapper {
    pub ctx: *mut DriverContext,
}

unsafe impl Send for HandlerWrapper {}
unsafe impl Sync for HandlerWrapper {}

impl acpi::Handler for HandlerWrapper {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let ctx = &mut *self.ctx;
        let res = &mut *ctx.res;

        let paddr_aligned = physical_address & !(PGSIZE - 1);
        let offset = physical_address - paddr_aligned;
        let size_aligned = (size + offset + PGSIZE - 1) & !(PGSIZE - 1);
        let pages = size_aligned / PGSIZE;

        // Alloc VA
        let va = ctx.va_allocator;
        ctx.va_allocator += size_aligned;

        // Alloc Slot
        let slot = CapPtr::from(ctx.slot_allocator);
        ctx.slot_allocator += 1;

        log!("Mapping ACPI: paddr={:#x}, pages={}, va={:#x}", paddr_aligned, pages, va);

        // Map using MMIO_CAP
        if let Err(e) = MMIO_CAP.get_frame(paddr_aligned, pages, slot) {
            panic!("Failed to map ACPI region {:#x} (pages={}): {:?}", paddr_aligned, pages, e);
        }
        let frame = Frame::from(slot);

        if let Err(e) = res.mmap(Badge::null(), frame, va, size_aligned) {
            panic!("Failed to mmap ACPI region: {:?}", e);
        }

        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new((va + offset) as *mut T).unwrap(),
            region_length: size,
            mapped_length: size_aligned,
            handler: self.clone(),
        }
    }

    fn unmap_physical_region<T>(region: &PhysicalMapping<Self, T>) {
        let ctx = unsafe { &mut *region.handler.ctx };
        let res = unsafe { &mut *ctx.res };

        let va = region.virtual_start.as_ptr() as usize;
        let va_aligned = va & !(PGSIZE - 1);
        res.munmap(Badge::null(), va_aligned, region.mapped_length).ok();
    }

    fn read_u8(&self, address: usize) -> u8 {
        let mapping = unsafe { self.map_physical_region::<u8>(address, 1) };
        let val = unsafe { *mapping.virtual_start.as_ptr() };
        acpi::Handler::unmap_physical_region(&mapping);
        val
    }
    fn read_u16(&self, address: usize) -> u16 {
        let mapping = unsafe { self.map_physical_region::<u16>(address, 2) };
        let val = unsafe { *mapping.virtual_start.as_ptr() };
        acpi::Handler::unmap_physical_region(&mapping);
        val
    }
    fn read_u32(&self, address: usize) -> u32 {
        let mapping = unsafe { self.map_physical_region::<u32>(address, 4) };
        let val = unsafe { *mapping.virtual_start.as_ptr() };
        acpi::Handler::unmap_physical_region(&mapping);
        val
    }
    fn read_u64(&self, address: usize) -> u64 {
        let mapping = unsafe { self.map_physical_region::<u64>(address, 8) };
        let val = unsafe { *mapping.virtual_start.as_ptr() };
        acpi::Handler::unmap_physical_region(&mapping);
        val
    }
    fn write_u8(&self, address: usize, value: u8) {
        let mapping = unsafe { self.map_physical_region::<u8>(address, 1) };
        unsafe { *mapping.virtual_start.as_ptr() = value };
        acpi::Handler::unmap_physical_region(&mapping);
    }
    fn write_u16(&self, address: usize, value: u16) {
        let mapping = unsafe { self.map_physical_region::<u16>(address, 2) };
        unsafe { *mapping.virtual_start.as_ptr() = value };
        acpi::Handler::unmap_physical_region(&mapping);
    }
    fn write_u32(&self, address: usize, value: u32) {
        let mapping = unsafe { self.map_physical_region::<u32>(address, 4) };
        unsafe { *mapping.virtual_start.as_ptr() = value };
        acpi::Handler::unmap_physical_region(&mapping);
    }
    fn write_u64(&self, address: usize, value: u64) {
        let mapping = unsafe { self.map_physical_region::<u64>(address, 8) };
        unsafe { *mapping.virtual_start.as_ptr() = value };
        acpi::Handler::unmap_physical_region(&mapping);
    }
    fn read_io_u8(&self, _port: u16) -> u8 {
        todo!()
    }
    fn read_io_u16(&self, _port: u16) -> u16 {
        todo!()
    }
    fn read_io_u32(&self, _port: u16) -> u32 {
        todo!()
    }
    fn write_io_u8(&self, _port: u16, _value: u8) {
        todo!()
    }
    fn write_io_u16(&self, _port: u16, _value: u16) {
        todo!()
    }
    fn write_io_u32(&self, _port: u16, _value: u32) {
        todo!()
    }
    fn read_pci_u8(&self, _address: acpi::PciAddress, _offset: u16) -> u8 {
        todo!()
    }
    fn read_pci_u16(&self, _address: acpi::PciAddress, _offset: u16) -> u16 {
        todo!()
    }
    fn read_pci_u32(&self, _address: acpi::PciAddress, _offset: u16) -> u32 {
        todo!()
    }
    fn write_pci_u8(&self, _address: acpi::PciAddress, _offset: u16, _value: u8) {
        todo!()
    }
    fn write_pci_u16(&self, _address: acpi::PciAddress, _offset: u16, _value: u16) {
        todo!()
    }
    fn write_pci_u32(&self, _address: acpi::PciAddress, _offset: u16, _value: u32) {
        todo!()
    }
    fn nanos_since_boot(&self) -> u64 {
        0
    }
    fn stall(&self, _nanoseconds: u64) {}
    fn sleep(&self, _milliseconds: u64) {}

    fn create_mutex(&self) -> acpi::Handle {
        acpi::Handle(0)
    }
    fn acquire(&self, _handle: acpi::Handle, _timeout: u16) -> Result<(), acpi::aml::AmlError> {
        Ok(())
    }
    fn release(&self, _handle: acpi::Handle) {}
}

impl aml::Handler for HandlerWrapper {
    fn read_u8(&self, address: usize) -> u8 {
        acpi::Handler::read_u8(self, address)
    }
    fn read_u16(&self, address: usize) -> u16 {
        acpi::Handler::read_u16(self, address)
    }
    fn read_u32(&self, address: usize) -> u32 {
        acpi::Handler::read_u32(self, address)
    }
    fn read_u64(&self, address: usize) -> u64 {
        acpi::Handler::read_u64(self, address)
    }
    fn write_u8(&mut self, address: usize, value: u8) {
        acpi::Handler::write_u8(self, address, value)
    }
    fn write_u16(&mut self, address: usize, value: u16) {
        acpi::Handler::write_u16(self, address, value)
    }
    fn write_u32(&mut self, address: usize, value: u32) {
        acpi::Handler::write_u32(self, address, value)
    }
    fn write_u64(&mut self, address: usize, value: u64) {
        acpi::Handler::write_u64(self, address, value)
    }
    fn read_io_u8(&self, _port: u16) -> u8 {
        0
    }
    fn read_io_u16(&self, _port: u16) -> u16 {
        0
    }
    fn read_io_u32(&self, _port: u16) -> u32 {
        0
    }
    fn write_io_u8(&self, _port: u16, _value: u8) {}
    fn write_io_u16(&self, _port: u16, _value: u16) {}
    fn write_io_u32(&self, _port: u16, _value: u32) {}
    fn read_pci_u8(&self, _segment: u16, _bus: u8, _device: u8, _function: u8, _offset: u16) -> u8 {
        0
    }
    fn read_pci_u16(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
    ) -> u16 {
        0
    }
    fn read_pci_u32(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
    ) -> u32 {
        0
    }
    fn write_pci_u8(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u8,
    ) {
    }
    fn write_pci_u16(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u16,
    ) {
    }
    fn write_pci_u32(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u32,
    ) {
    }
}
