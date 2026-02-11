#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;
use core::ptr::{read_volatile, write_volatile};
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::cap::{ENDPOINT_CAP, ENDPOINT_SLOT, MONITOR_CAP, RECV_SLOT, REPLY_SLOT};
use glenda::client::device::DeviceClient;
use glenda::client::{init, ResourceClient};
use glenda::error::Error;
use glenda::interface::{
    DeviceService, DriverService, ResourceService, SystemService, TimerDevice,
};
use glenda::ipc::{MsgTag, UTCB};
use glenda::mem::Perms;
use glenda::protocol;
use glenda::protocol::device::DeviceNode;
use glenda::protocol::resource::{DEVICE_ENDPOINT, INIT_ENDPOINT};

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => ({
        glenda::println!("Goldfish-RTC: {}", format_args!($($arg)*));
    })
}

const TIMER_TIME_LOW: usize = 0x00;
const TIMER_TIME_HIGH: usize = 0x04;
const TIMER_ALARM_LOW: usize = 0x08;
const TIMER_ALARM_HIGH: usize = 0x0C;
const TIMER_IRQ_ENABLED: usize = 0x10;
const TIMER_CLEAR_ALARM: usize = 0x14;
const TIMER_ALARM_STATUS: usize = 0x18;
const TIMER_CLEAR_INTERRUPT: usize = 0x1C;

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
}

impl TimerDevice for GoldfishRtc {
    fn get_time(&self) -> u64 {
        let low = self.read_reg(TIMER_TIME_LOW) as u64;
        let high = self.read_reg(TIMER_TIME_HIGH) as u64;
        (high << 32) | low
    }

    fn set_alarm(&mut self, timestamp: u64) -> Result<(), Error> {
        self.write_reg(TIMER_ALARM_LOW, (timestamp & 0xFFFFFFFF) as u32);
        self.write_reg(TIMER_ALARM_HIGH, (timestamp >> 32) as u32);
        self.write_reg(TIMER_IRQ_ENABLED, 1);
        Ok(())
    }

    fn stop_alarm(&mut self) -> Result<(), Error> {
        self.write_reg(TIMER_IRQ_ENABLED, 0);
        self.write_reg(TIMER_CLEAR_ALARM, 1);
        Ok(())
    }
}

pub struct RtcService {
    rtc: Option<GoldfishRtc>,
    endpoint: Endpoint,
    reply: Reply,
    recv: CapPtr,
    running: bool,

    device: &DeviceClient,
}

impl RtcService {
    pub fn new(device: &DeviceClient) -> Self {
        Self {
            rtc: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            device,
        }
    }
}

impl DriverService for RtcService {
    fn init(&mut self, node: DeviceNode) {
        let unicorn = Endpoint::from(CapPtr::from(11));
        let mmio_slot = 20;
        let tag = MsgTag::new(protocol::device::TIMER_PROTO, 4, glenda::ipc::MsgFlags::HAS_CAP);
        let args = [protocol::device::MAP_MMIO, node.id as usize, 0, mmio_slot, 0, 0, 0, 0];

        let mut utcb = unsafe { UTCB::new() };
        utcb.clear();
        utcb.set_msg_tag(tag);
        for i in 0..args.len() {
            utcb.set_mr(i, args[i]);
        }
        let _ = unicorn.call(&mut utcb);

        // Match what ns16550a does but different addr
        let mmio_va = 0x5000_2000;
        VSPACE_CAP
            .map(
                Frame::from(CapPtr::from(mmio_slot)),
                mmio_va,
                Perms::READ | Perms::WRITE | Perms::USER,
            )
            .expect("Failed to map RTC VSpace");

        self.rtc = Some(GoldfishRtc::new(mmio_va));
        glenda::println!("Goldfish RTC initialized at 0x{:x}", mmio_va);
    }
}

impl SystemService for RtcService {
    fn init(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(RECV_SLOT);

            if self.endpoint.recv(&mut utcb).is_ok() {
                if let Err(e) = self.dispatch(&mut utcb) {
                    utcb.set_msg_tag(MsgTag::err());
                    utcb.set_mr(0, e as usize);
                }
                let _ = self.reply(&mut utcb);
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let tag = utcb.get_msg_tag();

        let rtc = self.rtc.as_mut().ok_or(Error::NotInitialized)?;

        match tag.label() {
            protocol::device::timer::GET_TIME => {
                let time = rtc.get_time();
                utcb.set_mr(0, (time & 0xFFFFFFFF) as usize);
                utcb.set_mr(1, (time >> 32) as usize);
                utcb.set_msg_tag(MsgTag::ok());
                Ok(())
            }
            protocol::device::timer::SET_ALARM => {
                let low = utcb.get_mr(0) as u64;
                let high = utcb.get_mr(1) as u64;
                rtc.set_alarm((high << 32) | low)?;
                utcb.set_msg_tag(MsgTag::ok());
                Ok(())
            }
            protocol::device::timer::STOP_ALARM => {
                rtc.stop_alarm()?;
                utcb.set_msg_tag(MsgTag::ok());
                Ok(())
            }
            _ => Err(Error::NotSupported),
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}

#[unsafe(no_mangle)]
fn main() -> usize {
    let res_client = ResourceClient::new(MONITOR_CAP);
    let dev_cap = CapPtr::null();
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, dev_cap)
        .expect("Failed to get device endpoint cap");
    let dev_client = DeviceClient::new(Endpoint::from(dev_cap));
    let mut service = RtcService::new(&dev_client);
    res_client
        .alloc(Badge::null(), ResourceType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to allocate endpoint cap for service");
    service.listen(ENDPOINT_CAP, REPLY_SLOT);

    // Init from device node
    let node = DeviceNode {
        id: 0,
        compatible: alloc::string::String::from("google,goldfish-rtc"),
        base_addr: 0x101000,
        size: 0x1000,
        irq: 11,
        kind: glenda::utils::platform::DeviceKind::Timer,
        parent_id: None,
        children: alloc::vec::Vec::new(),
    };
    DriverService::init(&mut service, node);

    if let Err(e) = service.run() {
        glenda::println!("Goldfish RTC service failed: {:?}", e);
        return 1;
    }
    0
}
