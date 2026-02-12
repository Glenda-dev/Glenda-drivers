use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler};

pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const IRQ_SLOT: CapPtr = CapPtr::from(11);
pub const DMA_SLOT: CapPtr = CapPtr::from(12);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const MMIO_CAP: Frame = Frame::from(MMIO_SLOT);
pub const IRQ_CAP: IrqHandler = IrqHandler::from(IRQ_SLOT);

pub const MMIO_VA: usize = 0x6000_0000;
pub const DMA_VA: usize = 0x7000_0000;
