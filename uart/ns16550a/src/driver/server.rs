use crate::log;
use crate::UartService;
use glenda::cap::RECV_SLOT;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::error::Error;
use glenda::interface::SystemService;
use glenda::ipc::server::handle_call;
use glenda::ipc::{MsgTag, UTCB};
use glenda_drivers::interface::{DriverService, UartDriver};
use glenda_drivers::protocol;

impl<'a> SystemService for UartService<'a> {
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
        glenda::ipc_dispatch! {
            self, utcb,
            (protocol::UART_PROTO, protocol::uart::PUT_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    s.put_char(u.get_mr(0) as u8);
                    Ok(())
                })
            },
            (protocol::UART_PROTO, protocol::uart::GET_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    let c = s.get_char().ok_or(Error::NotFound)?;
                    Ok(c as usize)
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
