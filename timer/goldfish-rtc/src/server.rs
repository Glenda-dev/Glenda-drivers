use crate::log;
use crate::GoldfishRtc;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::DeviceClient;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda_drivers::interface::TimerDriver;
use glenda::interface::SystemService;
use glenda_drivers::interface::DriverService;
use glenda::ipc::{MsgTag, UTCB};
use glenda_drivers::protocol::timer::{GET_TIME, SET_ALARM, SET_TIME, STOP_ALARM};
use glenda_drivers::protocol::TIMER_PROTO;

pub struct RtcService<'a> {
    pub rtc: Option<GoldfishRtc>,
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
}

impl<'a> RtcService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            rtc: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }

    fn on_get_time(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let time = self.get_time();
        utcb.set_mr(0, time as usize);
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }

    fn on_set_time(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let timestamp = utcb.get_mr(0) as u64;
        self.set_time(timestamp)?;
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }

    fn on_set_alarm(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let timestamp = utcb.get_mr(0) as u64;
        self.set_alarm(timestamp)?;
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }

    fn on_stop_alarm(&mut self, _utcb: &mut UTCB) -> Result<(), Error> {
        self.stop_alarm()?;
        _utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }
}

impl<'a> TimerDriver for RtcService<'a> {
    fn get_time(&self) -> u64 {
        self.rtc.as_ref().map(|r| r.get_time()).unwrap_or(0)
    }

    fn set_time(&mut self, timestamp: u64) -> Result<(), Error> {
        if let Some(rtc) = &mut self.rtc {
            rtc.set_time(timestamp)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn set_alarm(&mut self, timestamp: u64) -> Result<(), Error> {
        if let Some(rtc) = &mut self.rtc {
            rtc.set_alarm(timestamp)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn stop_alarm(&mut self) -> Result<(), Error> {
        if let Some(rtc) = &mut self.rtc {
            rtc.stop_alarm()
        } else {
            Err(Error::NotInitialized)
        }
    }
}

impl<'a> SystemService for RtcService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        DriverService::init(self)
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        log!("RTC Service running...");
        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(self.recv);

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
        glenda::ipc_dispatch! {
            self, utcb,
            (TIMER_PROTO, GET_TIME) => Self::on_get_time,
            (TIMER_PROTO, SET_TIME) => Self::on_set_time,
            (TIMER_PROTO, SET_ALARM) => Self::on_set_alarm,
            (TIMER_PROTO, STOP_ALARM) => Self::on_stop_alarm,
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
