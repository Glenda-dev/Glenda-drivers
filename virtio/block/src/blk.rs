use core::ptr::NonNull;
use glenda::error::Error;
use glenda::interface::device::BlockDevice;
use virtio_common::{consts::*, VirtIOError, VirtIOTransport};

pub struct VirtIOBlk {
    transport: VirtIOTransport,
    // Add other fields like virtqueues here
}

impl VirtIOBlk {
    pub unsafe fn new(base: NonNull<u8>) -> Result<Self, VirtIOError> {
        let transport = VirtIOTransport::new(base)?;
        Ok(Self { transport })
    }

    pub fn init_hardware(&mut self) -> Result<(), VirtIOError> {
        self.transport.set_status(0); // Reset
        self.transport.add_status(STATUS_ACKNOWLEDGE);
        self.transport.add_status(STATUS_DRIVER);

        let features = self.transport.get_features();
        self.transport.set_features(features);

        self.transport.add_status(STATUS_FEATURES_OK);
        if self.transport.get_status() & STATUS_FEATURES_OK == 0 {
            return Err(VirtIOError::DeviceNotFound);
        }

        // Setup queues (TODO)

        self.transport.add_status(STATUS_DRIVER_OK);
        Ok(())
    }
}

impl BlockDevice for VirtIOBlk {
    fn capacity(&self) -> u64 {
        let cap_low = self.transport.read_config(0);
        let cap_high = self.transport.read_config(4);
        ((cap_high as u64) << 32) | (cap_low as u64)
    }

    fn block_size(&self) -> u32 {
        512
    }

    fn read_blocks(&mut self, _sector: u64, _buf: &mut [u8]) -> Result<usize, Error> {
        // Buffer transfer implementation depending on IPC mechanism
        Ok(0)
    }

    fn write_blocks(&mut self, _sector: u64, _buf: &[u8]) -> Result<usize, Error> {
        Ok(0)
    }

    fn sync(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
