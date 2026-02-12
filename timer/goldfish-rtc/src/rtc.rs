use core::ptr::{read_volatile, write_volatile};
use glenda::error::Error;

const TIMER_TIME_LOW: usize = 0x00;
const TIMER_TIME_HIGH: usize = 0x04;
const TIMER_ALARM_LOW: usize = 0x08;
const TIMER_ALARM_HIGH: usize = 0x0C;
const TIMER_IRQ_ENABLED: usize = 0x10;
const TIMER_CLEAR_ALARM: usize = 0x14;
const TIMER_ALARM_STATUS: usize = 0x18;
const TIMER_CLEAR_INTERRUPT: usize = 0x1C;
const TIMER_SET_TIME_LOW: usize = 0x20;
const TIMER_SET_TIME_HIGH: usize = 0x24;

pub struct GoldfishRtc {
    base: usize,
}

impl GoldfishRtc {
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    fn write_reg(&mut self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }

    pub fn ack_interrupt(&mut self) {
        self.write_reg(TIMER_CLEAR_INTERRUPT, 1);
    }

    pub fn is_alarm_triggered(&self) -> bool {
        self.read_reg(TIMER_ALARM_STATUS) != 0
    }

    pub fn get_time(&self) -> u64 {
        let low = self.read_reg(TIMER_TIME_LOW) as u64;
        let high = self.read_reg(TIMER_TIME_HIGH) as u64;
        let nanos = (high << 32) | low;
        nanos / 1_000_000_000
    }

    pub fn set_time(&mut self, timestamp: u64) -> Result<(), Error> {
        self.write_reg(TIMER_SET_TIME_LOW, (timestamp & 0xFFFFFFFF) as u32);
        self.write_reg(TIMER_SET_TIME_HIGH, (timestamp >> 32) as u32);
        Ok(())
    }

    pub fn set_alarm(&mut self, timestamp: u64) -> Result<(), Error> {
        self.write_reg(TIMER_ALARM_LOW, (timestamp & 0xFFFFFFFF) as u32);
        self.write_reg(TIMER_ALARM_HIGH, (timestamp >> 32) as u32);
        self.write_reg(TIMER_IRQ_ENABLED, 1);
        Ok(())
    }

    pub fn stop_alarm(&mut self) -> Result<(), Error> {
        self.write_reg(TIMER_IRQ_ENABLED, 0);
        self.write_reg(TIMER_CLEAR_ALARM, 1);
        Ok(())
    }
}
