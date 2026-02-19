use crate::sdhci::Sdhci;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::client::DeviceClient;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda_drivers::interface::SdioDriver;
use glenda::interface::SystemService;
use glenda_drivers::interface::DriverService;
use glenda::ipc::{MsgTag, UTCB};
use glenda::protocol::device::sdio::SdioCommand;
use glenda_drivers::protocol::sdio::SEND_COMMAND;
use glenda_drivers::protocol::SDIO_PROTO;

pub struct SdhciService<'a> {
    pub sdhci: Option<Sdhci>,
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,

    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
}

impl<'a> SdhciService<'a> {
    pub fn new(dev: &'a mut DeviceClient, res: &'a mut ResourceClient) -> Self {
        Self {
            sdhci: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            recv: CapPtr::null(),
            running: false,
            dev,
            res,
        }
    }

    fn on_send_command(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        // Deserialize SdioCommand from MRS
        let cmd = SdioCommand {
            cmd: utcb.get_mr(0) as u8,
            arg: utcb.get_mr(1) as u32,
            response_type: utcb.get_mr(2) as u8,
        };
        let resp = self.send_command(cmd)?;
        utcb.set_mr(0, resp[0] as usize);
        utcb.set_mr(1, resp[1] as usize);
        utcb.set_mr(2, resp[2] as usize);
        utcb.set_mr(3, resp[3] as usize);
        utcb.set_msg_tag(MsgTag::ok());
        Ok(())
    }
}

impl<'a> SdioDriver for SdhciService<'a> {
    fn send_command(&mut self, cmd: SdioCommand) -> Result<[u32; 4], Error> {
        if let Some(sdhci) = &mut self.sdhci {
            sdhci.send_command(cmd)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn read_blocks(&mut self, cmd: SdioCommand, buf: &mut [u8]) -> Result<(), Error> {
        if let Some(sdhci) = &mut self.sdhci {
            sdhci.read_blocks(cmd, buf)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn write_blocks(&mut self, cmd: SdioCommand, buf: &[u8]) -> Result<(), Error> {
        if let Some(sdhci) = &mut self.sdhci {
            sdhci.write_blocks(cmd, buf)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn set_bus_width(&mut self, width: u8) -> Result<(), Error> {
        if let Some(sdhci) = &mut self.sdhci {
            sdhci.set_bus_width(width)
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn set_clock(&mut self, hz: u32) -> Result<(), Error> {
        if let Some(sdhci) = &mut self.sdhci {
            sdhci.set_clock(hz)
        } else {
            Err(Error::NotInitialized)
        }
    }
}

impl<'a> SystemService for SdhciService<'a> {
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
            (SDIO_PROTO, SEND_COMMAND) => Self::on_send_command,
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
