use crate::driver::UartService;
use crate::layout::{IRQ_CAP, IRQ_SLOT, MMIO_SLOT, MMIO_VA};
use crate::log;
use crate::ns16550a::Ns16550a;
use glenda::cap::RECV_SLOT;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, SystemService, UartDevice};
use glenda::ipc::server::handle_call;
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::protocol;

impl<'a> SystemService for UartService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        let utcb = unsafe { UTCB::new() };

        // 1. Get MMIO Cap
        utcb.set_recv_window(MMIO_SLOT);
        let mmio = self.dev.get_mmio(Badge::null())?;

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, 0x1000)?;
        // 3. Get IRQ Cap
        utcb.set_recv_window(IRQ_SLOT);
        let irq_handler = self.dev.get_irq(Badge::null())?;
        // 4. Configure Interrupt
        // We use our endpoint to receive interrupts.
        // Note: Ideally we should use a badged endpoint to distinguish IRQ from IPC.
        // But for now we assume direct notification.
        irq_handler.set_notification(self.endpoint)?;
        irq_handler.set_priority(1)?;

        // 5. Init Hardware
        // IRQ is enabled by `init_hw`.
        let uart = Ns16550a::new(MMIO_VA, IRQ_CAP);
        uart.init_hw();
        self.uart = Some(uart);
        log!("Driver initialized!");
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
                match self.dispatch(&mut utcb) {
                    Ok(()) => {}
                    Err(e) => {
                        if e == Error::Success {
                            continue;
                        }
                        log!("Failed to dispatch message: {:?}", e);
                        utcb.set_msg_tag(MsgTag::err());
                        utcb.set_mr(0, e as usize);
                    }
                };
                self.reply(&mut utcb)?;
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge();
        let tag = utcb.get_msg_tag();
        let label = tag.label();
        let proto = tag.proto();
        let flags = tag.flags();
        let mrs = utcb.get_mrs();
        let size = utcb.get_size();
        log!(
            "Received message: badge={}, label={:#x}, proto={:#x}, flags={}, utcb.mrs_regs={:?}, size={}",
            badge,
            label,
            proto,
            flags,
            mrs,
            size
        );

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
            (protocol::PROCESS_PROTO, protocol::process::EXIT) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    s.running = false;
                    Ok(())
                })
            },
            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    if let Some(uart) = s.uart.as_mut() {
                        // 循环处理所有挂起的字符，直到硬件FIFO为空
                        loop {
                            match uart.handle_irq() {
                                Some(c) => uart.handle_char(c),
                                None => break,
                            }
                        }
                        // 必须回复内核 ACK，以重新启用该中断
                        uart.irq.ack()?;
                    }
                    Err::<(), _>(Error::Success)
                })
            },
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}
