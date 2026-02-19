use super::UartService;
use glenda_drivers::interface::UartDriver;

impl<'a> UartDriver for UartService<'a> {
    fn put_char(&mut self, c: u8) {
        if let Some(uart) = self.uart.as_mut() {
            uart.put_char(c);
        }
    }

    fn get_char(&mut self) -> Option<u8> {
        self.uart.as_mut().and_then(|u| u.get_char())
    }

    fn put_str(&mut self, s: &str) {
        if let Some(uart) = self.uart.as_mut() {
            uart.put_str(s);
        }
    }
}
