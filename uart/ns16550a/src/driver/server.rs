use crate::layout::RING_VA;
use crate::UartService;
use glenda::cap::RECV_SLOT;
use glenda::cap::{CapPtr, Endpoint, Frame, Reply};
use glenda::ipc::Badge;
use glenda::error::Error;
use glenda::interface::memory::MemoryService;
use glenda::interface::resource::ResourceService;
use glenda::interface::SystemService;
use glenda::io::uring::{IoUringBuffer, IoUringServer};
use glenda::ipc::server::{handle_call, handle_cap_call};
use glenda::ipc::{MsgTag, UTCB};
use glenda::utils::manager::CSpaceService;
use glenda_drivers::interface::{DriverService, UartDriver};
use glenda_drivers::protocol;

impl<'a> UartService<'a> {
    fn setup_ring(&mut self, sq: u32, cq: u32, notify_ep: Endpoint) -> Result<Frame, Error> {
        let slot = self.cspace.alloc(self.res)?;
        let (_paddr, frame): (usize, Frame) = self.res.dma_alloc(Badge::null(), 1, slot)?;

        self.res.mmap(Badge::null(), frame.clone(), RING_VA, 4096)?;
        
        let ring = unsafe { IoUringBuffer::new(RING_VA as *mut u8, 4096, sq, cq) };
        let mut server = IoUringServer::new(ring);
        server.set_client_notify(notify_ep);

        if let Some(uart) = self.uart.as_mut() {
            uart.ring = Some(server);
        }

        Ok(frame)
    }

    fn setup_shm(&mut self, frame: Frame, vaddr: usize, paddr: u64, size: usize) -> Result<(), Error> {
        if let Some(uart) = self.uart.as_mut() {
            uart.setup_shm(frame, vaddr, paddr, size)
        } else {
            Err(Error::NotInitialized)
        }
    }
}

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
                        let badge = utcb.get_badge();
                        let proto = utcb.get_msg_tag().proto();
                        let label = utcb.get_msg_tag().label();
                        error!(
                            "Failed to dispatch message for {}: {:?}, proto={:#x}, label={:#x}",
                            badge, e, proto, label
                        );
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
            (protocol::UART_PROTO, protocol::uart::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                let recv_slot = s.recv;
                let slot = s.cspace.alloc(s.res)?;
                let sq = u.get_mr(0) as u32;
                let cq = u.get_mr(1) as u32;

                s.cspace.root().move_cap(recv_slot, slot)?;

                handle_cap_call(u, |_u| {
                    let notify_ep = Endpoint::from(slot);
                    let frame = s.setup_ring(sq, cq, notify_ep)?;
                    Ok(frame.cap())
                })
            },
            (protocol::UART_PROTO, protocol::uart::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                let recv_slot = s.recv;
                let slot = s.cspace.alloc(s.res)?;
                let vaddr = u.get_mr(0);
                let size = u.get_mr(1);
                let paddr = u.get_mr(2) as u64;

                s.cspace.root().move_cap(recv_slot, slot)?;

                handle_call(u, |_u| {
                    let frame = Frame::from(slot);
                    s.setup_shm(frame, vaddr, paddr, size)?;
                    Ok(0usize)
                })
            },
            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_u| {
                    if let Some(uart) = s.uart.as_mut() {
                        // 1. 处理异步IO请求 (SQE)
                        uart.handle_async_io();

                        // 2. 处理硬件中断 (RX)
                        loop {
                            match uart.handle_irq() {
                                Some(c) => uart.handle_char(c),
                                None => break,
                            }
                        }
                        // 3. 再次处理异步IO (可能刚收到数据完成了READ)
                        uart.handle_async_io();

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
