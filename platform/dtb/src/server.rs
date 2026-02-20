use crate::driver::{DtbDriver, PowerMethod};
use crate::log;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::error::Error;
use glenda::interface::{DeviceService, SystemService};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::ipc_dispatch;
use glenda::protocol::device::{LogicDeviceDesc, LogicDeviceType};
use glenda_drivers::interface::DriverService;
use glenda_drivers::protocol;

impl SystemService for DtbDriver {
    fn init(&mut self) -> Result<(), Error> {
        // 1. Initialize hardware parsing (map FDT, etc.)
        DriverService::init(self)?;

        // 2. Probe sub-devices and report to Device Manager (Unicorn)
        log!("Probing devices...");
        let devices = self.probe()?;
        self.dev_client.report(Badge::null(), devices)?;

        // 3. Register logic devices (Platform/Power and Thermal)
        if self.has_power_off || self.has_reboot {
            let platform_desc = LogicDeviceDesc {
                dev_type: LogicDeviceType::Platform,
                parent_name: "dtb".into(),
                badge: Some(1),
            };
            log!("Registering platform logic...");
            self.dev_client.register_logic(Badge::null(), platform_desc, self.endpoint.cap())?;
        }

        if !self.thermal_zones.zones.is_empty() {
            let thermal_desc = LogicDeviceDesc {
                dev_type: LogicDeviceType::Thermal,
                parent_name: "dtb".into(),
                badge: Some(1),
            };
            log!("Registering thermal logic...");
            self.dev_client.register_logic(Badge::null(), thermal_desc, self.endpoint.cap())?;
        }

        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(glenda::cap::REPLY_SLOT);
            utcb.set_recv_window(glenda::cap::RECV_SLOT);

            self.endpoint.recv(utcb)?;

            if let Err(e) = self.dispatch(utcb) {
                utcb.set_msg_tag(MsgTag::err());
                utcb.set_mr(0, e as usize);
            }

            let _ = self.reply(utcb);
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        ipc_dispatch! {
            self, utcb,
            (protocol::PLATFORM_PROTO, protocol::platform::SYSTEM_OFF) => DtbDriver::handle_off,
            (protocol::PLATFORM_PROTO, protocol::platform::SYSTEM_RESET) => DtbDriver::handle_reboot,
            (protocol::THERMAL_PROTO, protocol::thermal::GET_TEMPERATURE) => DtbDriver::handle_get_temperature,
            (protocol::THERMAL_PROTO, protocol::thermal::GET_ZONE_COUNT) => DtbDriver::handle_get_zones_count,
            (protocol::THERMAL_PROTO, protocol::thermal::GET_ZONE_INFO) => DtbDriver::handle_get_zones_info,
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb).map_err(|e| e.into())
    }

    fn stop(&mut self) {
        self.running = false;
    }
}

impl DtbDriver {
    pub fn handle_off(&mut self, _utcb: &mut UTCB) -> Result<(), Error> {
        if !self.has_power_off {
            return Err(Error::NotSupported);
        }
        log!("System off via {:?}", self.power_method);
        match self.power_method {
            PowerMethod::Sbi => {
                // SBI system_reset with type=SHUTDOWN
                Ok(())
            }
            PowerMethod::Psci => {
                // PSCI system_off
                Ok(())
            }
            _ => Err(Error::NotSupported),
        }
    }

    pub fn handle_reboot(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        if !self.has_reboot {
            return Err(Error::NotSupported);
        }
        let reset_type = utcb.get_mr(0);
        log!("System reboot (type={}) via {:?}", reset_type, self.power_method);
        match self.power_method {
            PowerMethod::Sbi => Ok(()),
            PowerMethod::Psci => Ok(()),
            PowerMethod::Syscon => Ok(()),
            _ => Err(Error::NotSupported),
        }
    }

    pub fn handle_get_temperature(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let sensor_id: usize = utcb.get_mr(0);
        let temp = if sensor_id < self.thermal_zones.zones.len() {
            // Placeholder: real reading would use self.thermal_base or similar
            450 + (sensor_id * 10) as u32
        } else {
            0
        };
        utcb.set_mr(0, temp as usize);
        utcb.set_msg_tag(MsgTag::new(0, 1, glenda::ipc::MsgFlags::NONE));
        Ok(())
    }

    pub fn handle_get_zones_count(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        utcb.set_mr(0, self.thermal_zones.zones.len());
        utcb.set_msg_tag(MsgTag::new(0, 1, glenda::ipc::MsgFlags::NONE));
        Ok(())
    }

    pub fn handle_get_zones_info(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let index: usize = utcb.get_mr(0);
        if let Some(_zone) = self.thermal_zones.zones.get(index) {
            unsafe {
                let size = utcb.write_postcard(_zone)?;
                utcb.set_msg_tag(MsgTag::new(0, (size + 7) / 8, glenda::ipc::MsgFlags::NONE));
            }
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }
}
