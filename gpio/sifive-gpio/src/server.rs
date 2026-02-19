use crate::SiFiveGpio;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::DeviceClient;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda_drivers::interface::GpioDriver;
use glenda::interface::SystemService;
use glenda_drivers::interface::DriverService;
use glenda::ipc::{MsgTag, UTCB};
use glenda_drivers::protocol::gpio::{READ, SET_MODE, WRITE};
use glenda_drivers::protocol::GPIO_PROTO;

pub struct GpioService<'a> {
    pub gpio: Option<SiFiveGpio>,
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
}

impl<'a> GpioService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            gpio: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }

    fn on_set_mode(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let pin = utcb.get_mr(0) as u32;
        let mode = utcb.get_mr(1) as u8;
        self.set_mode(pin, mode)?;
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }

    fn on_write(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let pin = utcb.get_mr(0) as u32;
        let value = utcb.get_mr(1) != 0;
        self.write(pin, value)?;
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }

    fn on_read(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let pin = utcb.get_mr(0) as u32;
        let value = self.read(pin)?;
        utcb.set_mr(0, value as usize);
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }
}

impl<'a> GpioDriver for GpioService<'a> {
    fn set_mode(&mut self, pin: u32, mode: u8) -> Result<(), Error> {
        if let Some(gpio) = &mut self.gpio {
            gpio.set_mode(pin, mode)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn write(&mut self, pin: u32, value: bool) -> Result<(), Error> {
        if let Some(gpio) = &mut self.gpio {
            gpio.write(pin, value)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn read(&self, pin: u32) -> Result<bool, Error> {
        if let Some(gpio) = &self.gpio {
            gpio.read(pin)
        } else {
            Err(Error::NotInitialized)
        }
    }
}

impl<'a> SystemService for GpioService<'a> {
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
            (GPIO_PROTO, SET_MODE) => Self::on_set_mode,
            (GPIO_PROTO, WRITE) => Self::on_write,
            (GPIO_PROTO, READ) => Self::on_read,
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
