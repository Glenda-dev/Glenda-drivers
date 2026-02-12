use core::ptr::{read_volatile, write_volatile};
use glenda::error::Error;
use glenda::protocol::device::gpio::*;

const GPIO_VALUE: usize = 0x00;
const GPIO_INPUT_EN: usize = 0x04;
const GPIO_OUTPUT_EN: usize = 0x08;
const GPIO_PULLUP_EN: usize = 0x0C;

pub struct SiFiveGpio {
    base: usize,
}

impl SiFiveGpio {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    fn write_reg(&mut self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }

    pub fn set_mode(&mut self, pin: u32, mode: u8) -> Result<(), Error> {
        let bit = 1 << pin;
        match mode {
            MODE_INPUT => {
                let val = self.read_reg(GPIO_INPUT_EN);
                self.write_reg(GPIO_INPUT_EN, val | bit);
                let val = self.read_reg(GPIO_OUTPUT_EN);
                self.write_reg(GPIO_OUTPUT_EN, val & !bit);
            }
            MODE_OUTPUT => {
                let val = self.read_reg(GPIO_OUTPUT_EN);
                self.write_reg(GPIO_OUTPUT_EN, val | bit);
                let val = self.read_reg(GPIO_INPUT_EN);
                self.write_reg(GPIO_INPUT_EN, val & !bit);
            }
            _ => return Err(Error::InvalidArgs),
        }
        Ok(())
    }

    pub fn write(&mut self, pin: u32, value: bool) -> Result<(), Error> {
        let bit = 1 << pin;
        let mut val = self.read_reg(GPIO_VALUE);
        if value {
            val |= bit;
        } else {
            val &= !bit;
        }
        self.write_reg(GPIO_VALUE, val);
        Ok(())
    }

    pub fn read(&self, pin: u32) -> Result<bool, Error> {
        let val = self.read_reg(GPIO_VALUE);
        Ok((val & (1 << pin)) != 0)
    }
}
