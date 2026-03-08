use super::UartService;
use glenda::drivers::interface::UartDriver;
use glenda::error::Error;

impl<'a> UartDriver for UartService<'a> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if let Some(uart) = self.uart.as_mut() {
            uart.write(buf)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if let Some(uart) = self.uart.as_mut() {
            uart.read(buf)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn set_baud_rate(&mut self, baud: u32) {
        if let Some(uart) = self.uart.as_mut() {
            uart.set_baud_rate(baud);
        }
    }
}
