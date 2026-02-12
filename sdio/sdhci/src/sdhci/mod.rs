use core::ptr::{read_volatile, write_volatile};
use glenda::error::Error;
use glenda::protocol::device::sdio::*;

pub struct Sdhci {
    base: usize,
}

impl Sdhci {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    fn write_reg(&mut self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }

    pub fn send_command(&mut self, _cmd: SdioCommand) -> Result<[u32; 4], Error> {
        // SDHCI Command sequence:
        // 1. Argument -> SDHCI_ARGUMENT
        // 2. Command -> SDHCI_COMMAND (This triggers the command)
        // 3. Wait for complete
        // 4. Read Response registers
        Ok([0; 4])
    }

    pub fn read_blocks(&mut self, _cmd: SdioCommand, _buf: &mut [u8]) -> Result<(), Error> {
        Ok(())
    }

    pub fn write_blocks(&mut self, _cmd: SdioCommand, _buf: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    pub fn set_bus_width(&mut self, _width: u8) -> Result<(), Error> {
        Ok(())
    }

    pub fn set_clock(&mut self, _hz: u32) -> Result<(), Error> {
        Ok(())
    }
}
