use crate::layout::*;
use crate::UartService;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply, CSPACE_CAP};
use glenda::error::Error;
use glenda::interface::SystemService;
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::drivers::interface::{DriverService, UartDriver};
use glenda::drivers::protocol;

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
            utcb.set_recv_window(self.recv);
            match self.endpoint.recv(&mut utcb) {
                Ok(_) => {}
                Err(e) => {
                    error!("Recv error: {:?}", e);
                    continue;
                }
            };

            let badge = utcb.get_badge();
            let proto = utcb.get_msg_tag().proto();
            let label = utcb.get_msg_tag().label();

            let res = self.dispatch(&mut utcb);
            if let Err(e) = res {
                if e == Error::Success {
                    continue;
                }
                error!(
                    "Failed to dispatch message for {}: {:?}, proto={:#x}, label={:#x}",
                    badge, e, proto, label
                );
                utcb.set_msg_tag(MsgTag::err());
                utcb.set_mr(0, e as usize);
            }

            if let Err(e) = self.reply(&mut utcb) {
                error!("Reply failed: {:?}", e);
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let badge = utcb.get_badge().bits();
        if badge != 0 && self.connected_client.is_some() && self.connected_client != Some(badge) {
            let proto = utcb.get_msg_tag().proto();
            if proto != glenda::protocol::KERNEL_PROTO {
                return Err(Error::PermissionDenied);
            }
        }

        let res = glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::DEVICE_PROTO, label) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    use glenda::interface::device::DeviceService;
                    use glenda::protocol::device::*;
                    match label {
                        GET_MMIO => {
                            let id = u.get_mr(0);
                            let recv = u.get_recv_window();
                            let (frame, addr, size) = s.dev.get_mmio(Badge::null(), id, recv)?;
                            u.set_mr(0, addr);
                            u.set_mr(1, size);
                            Ok(frame.cap())
                        }
                        GET_IRQ => {
                            let id = u.get_mr(0);
                            let recv = u.get_recv_window();
                            let irq = s.dev.get_irq(Badge::null(), id, recv)?;
                            Ok(irq.cap())
                        }
                        _ => Err(Error::NotSupported),
                    }
                })
            },
            (protocol::UART_PROTO, protocol::uart::PUT_CHAR) => |s: &mut Self, u: &mut UTCB| {
                if badge != 0 && s.connected_client.is_none() {
                    s.connected_client = Some(badge);
                }
                handle_call(u, |u| {
                    if let Some(uart) = s.uart.as_mut() {
                        uart.put_char(u.get_mr(0) as u8);
                    }
                    Ok(())
                })
            },
            (protocol::UART_PROTO, protocol::uart::GET_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    if let Some(uart) = s.uart.as_mut() {
                        let c = uart.get_char().ok_or(Error::NotFound)?;
                        Ok(c as usize)
                    } else {
                        Err(Error::NotInitialized)
                    }
                })
            },
            (protocol::UART_PROTO, protocol::uart::PUT_STR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    if let Some(uart) = s.uart.as_mut() {
                        let len = u.get_mr(0);
                        let s_bytes = unsafe { core::slice::from_raw_parts(u.mr_ptr(1) as *const u8, len) };
                        if let Ok(s_str) = core::str::from_utf8(s_bytes) {
                            uart.put_str(s_str);
                        }
                    }
                    Ok(())
                })
            },
            (protocol::UART_PROTO, protocol::uart::SET_BAUD_RATE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    if let Some(uart) = s.uart.as_mut() {
                        uart.set_baud_rate(u.get_mr(0) as u32);
                    }
                    Ok(())
                })
            },
            (protocol::UART_PROTO, protocol::uart::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                handle_cap_call(u, |u| {
                    let recv_slot = s.recv;
                    let slot = RING_SLOT;
                    let sq = u.get_mr(0) as u32;
                    let cq = u.get_mr(1) as u32;
                    // Move cap to predefined slot
                    CSPACE_CAP.move_cap(recv_slot, slot)?;
                    let notify_ep = Endpoint::from(slot);
                    let frame = s.setup_ring(sq, cq, notify_ep)?;
                    Ok(frame.cap())
                })
            },
            (protocol::UART_PROTO, protocol::uart::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let recv_slot = s.recv;
                    let slot = SHM_SLOT;
                    let vaddr = u.get_mr(0);
                    let size = u.get_mr(1);
                    let paddr = u.get_mr(2) as u64;
                    CSPACE_CAP.move_cap(recv_slot, slot)?;
                    let frame = Frame::from(slot);
                    s.setup_shm(frame, vaddr, paddr, size)?;
                    Ok(())
                })
            },
            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |u| {
                    if let Some(_uart) = s.uart.as_mut() {
                        let bits = u.get_badge().bits();

                        let is_irq = bits & IRQ_BADGE != 0;
                        let is_cq = bits & glenda::io::uring::NOTIFY_IO_URING_CQ != 0;
                        let is_sq = bits & glenda::io::uring::NOTIFY_IO_URING_SQ != 0;

                        if is_irq {
                            if let Err(e) = s.handle_notify_irq() {
                                log!("IRQ failed: {:?}", e);
                            }
                        }

                    // 2. Check for CQ completion notifications
                        if is_cq {
                            if let Err(e) = s.handle_notify_cq() {
                                error!("CQ notify failed: {:?}", e);
                            }
                        }

                    // 3. Check for SQ submission notifications
                        if is_sq {
                            if let Err(e) = s.handle_notify_sq() {
                                error!("SQ notify failed: {:?}", e);
                            }
                        }
                    }
                    Ok(())
                })
            },
        };
        res
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.running = false;
    }
}

impl<'a> UartService<'a> {
    fn handle_notify_irq(&mut self) -> Result<(), Error> {
        if let Some(uart) = self.uart.as_mut() {
            loop {
                match uart.handle_irq() {
                    Some(c) => uart.handle_char(c),
                    None => break,
                }
            }

            // 必须回复内核 ACK，以重新启用该中断
            if let Err(e) = uart.irq.ack() {
                error!("Failed to ACK UART IRQ: {:?}", e);
            }
        }
        Ok(())
    }

    fn handle_notify_cq(&mut self) -> Result<(), Error> {
        log!("UART: Notify CQ");
        // CQ (Completion Queue) notification from client means client has consumed some entries.
        if let Some(uart) = self.uart.as_mut() {
            uart.handle_cq();
        }
        Ok(())
    }

    fn handle_notify_sq(&mut self) -> Result<(), Error> {
        log!("UART: Notify SQ");
        // SQ (Submission Queue) notification logic:
        // Client has submitted new requests.
        if let Some(uart) = self.uart.as_mut() {
            uart.handle_sq();
        }
        Ok(())
    }
}
