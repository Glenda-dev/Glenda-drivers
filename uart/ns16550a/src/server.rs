use crate::Ns16550a;
use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler, Reply, RECV_SLOT, VSPACE_CAP};
use glenda::error::Error;
use glenda::interface::device::UartDevice;
use glenda::interface::{DriverService, SystemService};
use glenda::ipc::server::handle_call;
use glenda::ipc::{MsgTag, UTCB};
use glenda::mem::Perms;
use glenda::protocol;
use glenda::protocol::device::DeviceNode;

pub struct UartService {
    uart: Option<Ns16550a>,
    endpoint: Endpoint,
    reply: Reply,
    irq_ep: Endpoint,
    running: bool,
}

impl UartService {
    pub fn new() -> Self {
        Self {
            uart: None,
            endpoint: Endpoint::from(CapPtr::null()),
            reply: Reply::from(CapPtr::null()),
            irq_ep: Endpoint::from(CapPtr::null()),
            running: false,
        }
    }
}

impl DriverService for UartService {
    fn init(&mut self, node: DeviceNode) {
        log!("Initializing for device: {}", node.compatible);

        // 1. Map MMIO
        let unicorn = Endpoint::from(CapPtr::from(11));
        let mmio_slot = 20;
        let tag = MsgTag::new(
            protocol::device::UART_PROTO,
            protocol::device::MAP_MMIO,
            glenda::ipc::MsgFlags::HAS_CAP,
        );

        let mut utcb = unsafe { UTCB::new() };
        utcb.clear();
        utcb.set_msg_tag(tag);
        utcb.set_mr(0, node.id as usize);
        utcb.set_mr(1, 0);
        utcb.set_mr(2, mmio_slot);

        if unicorn.call(&mut utcb).is_err() || utcb.get_mr(0) != 0 {
            log!("Failed to map MMIO");
            return;
        }

        let mmio_va = 0x5000_0000;
        VSPACE_CAP
            .map(
                Frame::from(CapPtr::from(mmio_slot)),
                mmio_va,
                Perms::READ | Perms::WRITE | Perms::USER,
            )
            .expect("Failed to map VSpace");

        // 2. Get IRQ
        let irq_slot = 21;
        let tag = MsgTag::new(
            protocol::device::UART_PROTO,
            protocol::device::GET_IRQ,
            glenda::ipc::MsgFlags::HAS_CAP,
        );
        utcb.clear();
        utcb.set_msg_tag(tag);
        utcb.set_mr(0, node.id as usize);
        utcb.set_mr(1, 0);
        utcb.set_mr(2, irq_slot);

        if unicorn.call(&mut utcb).is_err() || utcb.get_mr(0) != 0 {
            log!("Failed to get IRQ");
            return;
        }

        // 3. Allocate IRQ endpoint
        let irq_ep_slot = 22;
        let warren = Endpoint::from(CapPtr::from(10));
        let tag = MsgTag::new(protocol::PROCESS_PROTO, 1, glenda::ipc::MsgFlags::NONE); // ALLOC_CAP
        utcb.clear();
        utcb.set_msg_tag(tag);
        let _ = warren.call(&mut utcb);

        let irq_ep = Endpoint::from(CapPtr::from(irq_ep_slot));
        let uart = Ns16550a::new(mmio_va, IrqHandler::from(CapPtr::from(irq_slot)));
        uart.init_hw();
        uart.irq.set_notification(irq_ep).expect("Failed to set IRQ notification");
        uart.irq.ack().expect("Failed to ack IRQ");

        self.uart = Some(uart);
        self.irq_ep = irq_ep;

        log!("UART hardware initialized");
    }
}

impl UartDevice for UartService {
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

impl SystemService for UartService {
    fn init(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = Reply::from(reply);
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.running = true;
        log!("UART Service running...");

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
        glenda::ipc_dispatch! {
            self, utcb,
            (protocol::device::UART_PROTO, protocol::device::uart::PUT_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    s.put_char(u.get_mr(0) as u8);
                    Ok(())
                })
            },
            (protocol::device::UART_PROTO, protocol::device::uart::GET_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    let c = s.get_char().ok_or(Error::NotFound)?;
                    Ok(c as usize)
                })
            },
            (protocol::PROCESS_PROTO, protocol::process::EXIT) => |s: &mut Self, _u: &mut UTCB| {
                s.running = false;
                Ok(())
            }
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
