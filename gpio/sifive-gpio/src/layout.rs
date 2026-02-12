use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler};

pub const MMIO_SLOT: CapPtr = CapPtr::from(10);
pub const MMIO_CAP: Frame = Frame::from(MMIO_SLOT);
pub const MMIO_VA: usize = 0x80000000;

pub const IRQ_SLOT: CapPtr = CapPtr::from(11);
pub const IRQ_CAP: IrqHandler = IrqHandler::from(IRQ_SLOT);

pub const DEVICE_SLOT: CapPtr = CapPtr::from(14);
pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
