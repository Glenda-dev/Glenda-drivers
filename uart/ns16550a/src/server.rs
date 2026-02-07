use crate::Ns16550a;
use glenda::cap::{CapPtr, Endpoint, Frame, IrqHandler, Reply, VSPACE_CAP};
use glenda::error::Error;
use glenda::interface::device::{DriverService, UartDevice};
use glenda::interface::system::SystemService;
use glenda::ipc::{Badge, MsgArgs, MsgFlags, MsgTag, UTCB};
use glenda::manager::device::DeviceNode;
use glenda::mem::Perms;
use glenda::protocol;

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
        let tag = MsgTag::new(protocol::device::UART_PROTO, 4, glenda::ipc::MsgFlags::HAS_CAP);
        let args = [protocol::device::MAP_MMIO, node.id, 0, mmio_slot, 0, 0, 0, 0];

        let _ = unicorn.call(tag, args);
        if unsafe { UTCB::get() }.mrs_regs[0] != 0 {
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
        let tag = MsgTag::new(protocol::device::UART_PROTO, 4, glenda::ipc::MsgFlags::HAS_CAP);
        let args = [protocol::device::GET_IRQ, node.id, 0, irq_slot, 0, 0, 0, 0];
        let _ = unicorn.call(tag, args);
        if unsafe { UTCB::get() }.mrs_regs[0] != 0 {
            log!("Failed to get IRQ");
            return;
        }

        // 3. Allocate IRQ endpoint
        let irq_ep_slot = 22;
        let warren = Endpoint::from(CapPtr::from(10));
        let tag = MsgTag::new(protocol::PROCESS_PROTO, 1, glenda::ipc::MsgFlags::NONE); // ALLOC_CAP
        let _ = warren.call(tag, [0; 8]); // Dummy for now

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
            // Need to multiplex between IPC endpoint and IRQ endpoint
            // For now, we use a simple poll or separate thread if supported.
            // In Glenda microkernel, we usually use Recv on the service endpoint.
            // If we have notifications, we might receive 0 badge or special badge.

            let badge_bits = self.endpoint.recv(self.reply.cap())?;
            let badge = Badge::new(badge_bits);
            let utcb = unsafe { UTCB::get() };
            let tag = utcb.msg_tag;
            let args = utcb.mrs_regs;

            let res = self.dispatch(badge, tag.label(), tag.proto(), tag.flags(), args);
            match res {
                Ok(ret) => self.reply(protocol::GENERIC_PROTO, 0, MsgFlags::OK, ret)?,
                Err(e) => {
                    self.reply(protocol::GENERIC_PROTO, 0, MsgFlags::ERROR, [e as usize; 8])?
                }
            }
        }
        Ok(())
    }

    fn dispatch(
        &mut self,
        _badge: Badge,
        label: usize,
        proto: usize,
        _flags: MsgFlags,
        msg: MsgArgs,
    ) -> Result<MsgArgs, Error> {
        let uart = self.uart.as_mut().ok_or(Error::NotInitialized)?;

        match proto {
            protocol::device::UART_PROTO => match label {
                protocol::device::uart::PUT_CHAR => {
                    uart.put_char(msg[0] as u8);
                    Ok([0; 8])
                }
                protocol::device::uart::GET_CHAR => {
                    if let Some(c) = uart.get_char() {
                        Ok([c as usize, 0, 0, 0, 0, 0, 0, 0])
                    } else {
                        Err(Error::NotFound)
                    }
                }
                _ => Err(Error::NotImplemented),
            },
            _ => Err(Error::InvalidProtocol),
        }
    }

    fn reply(
        &mut self,
        _label: usize,
        _proto: usize,
        flags: MsgFlags,
        msg: MsgArgs,
    ) -> Result<(), Error> {
        let utcb = unsafe { UTCB::get() };
        utcb.msg_tag = MsgTag::new(protocol::GENERIC_PROTO, msg.len(), flags);
        utcb.mrs_regs = msg;
        self.reply.reply(utcb.msg_tag, utcb.mrs_regs)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
