use crate::RamdiskService;
use crate::driver::Ramdisk;
use crate::layout::{BUFFER_SLOT, MMIO_SLOT, MMIO_VA, NOTIFY_SLOT};
use glenda::cap::{CSPACE_CAP, CapPtr, ENDPOINT_SLOT, Endpoint, RECV_SLOT, Reply};
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::interface::{DeviceService, MemoryService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda_drivers::protocol::BLOCK_PROTO;

impl<'a> SystemService for RamdiskService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");

        // 1. Get MMIO Cap (backing store)=
        let (mmio, paddr, size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        log!("Got memory region: paddr={:#x}, size={:#x}", paddr, size);

        // 2. Map MMIO
        self.res.mmap(Badge::null(), mmio, MMIO_VA, size)?;

        // 3. Init hardware (ramdisk logic)
        let data = unsafe { core::slice::from_raw_parts_mut(MMIO_VA as *mut u8, size) };
        let mut ramdisk = Ramdisk::new(data);
        ramdisk.set_block_size(4096);
        log!(
            "Initialized Ramdisk with {} blocks ({} bytes each)",
            ramdisk.capacity(),
            ramdisk.block_size()
        );
        self.ramdisk = Some(ramdisk);

        // 4. Register logical device to Unicorn
        let desc = glenda::protocol::device::LogicDeviceDesc {
            name: alloc::string::String::from("ramdisk"),
            dev_type: glenda::protocol::device::LogicDeviceType::Block,
            parent_name: alloc::string::String::from("ramdisk"),
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, ENDPOINT_SLOT)?;

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
        log!("Listening for requests...");

        while self.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply.cap());
            utcb.set_recv_window(RECV_SLOT);

            if self.endpoint.recv(&mut utcb).is_ok() {
                match SystemService::dispatch(self, &mut utcb) {
                    Ok(_) => {
                        let _ = SystemService::reply(self, &mut utcb);
                    }
                    Err(Error::Success) => {
                        // Successfully handled, but no reply needed (e.g., notification)
                    }
                    Err(e) => {
                        let badge = utcb.get_badge();
                        let tag = utcb.get_msg_tag();
                        error!(
                            "Dispatch error: {:?} badge={}, proto={:#x}, label={:#x}",
                            e,
                            badge,
                            tag.proto(),
                            tag.label()
                        );
                        utcb.set_msg_tag(MsgTag::err());
                        utcb.set_mr(0, e as usize);
                        let _ = SystemService::reply(self, &mut utcb);
                    }
                }
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_| {
                    if let Some(ramdisk) = s.ramdisk.as_mut() {
                        ramdisk.handle_io()?;
                    }
                    Ok(())
                })
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::GET_CAPACITY) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.ramdisk.as_ref().unwrap().capacity() as usize))
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::GET_BLOCK_SIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| Ok(s.ramdisk.as_ref().unwrap().block_size() as usize))
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                let recv_slot = s.recv;
                let client_vaddr = u.get_mr(0);
                let size = u.get_mr(1);
                let paddr = u.get_mr(2) as u64;

                // Move capabilities after reading registers
                CSPACE_CAP.move_cap(recv_slot, BUFFER_SLOT)?;

                handle_call(u, |_| {
                    let res = unsafe { &mut *(s.res as *mut ResourceClient) };
                    s.ramdisk.as_mut().unwrap().setup_buffer(res, client_vaddr, size, paddr)?;
                    Ok(0usize)
                })
            },
            (BLOCK_PROTO, glenda_drivers::protocol::block::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                let recv_slot = s.recv;
                let sq = u.get_mr(0) as u32;
                let cq = u.get_mr(1) as u32;

                // Move capabilities after reading registers
                CSPACE_CAP.move_cap(recv_slot, NOTIFY_SLOT)?;

                handle_cap_call(u, |_| {
                    // Transfer notification endpoint
                    let res = unsafe { &mut *(s.res as *mut ResourceClient) };
                    let notify_ep = Endpoint::from(NOTIFY_SLOT);

                    let ramdisk = s.ramdisk.as_mut().unwrap();
                    let frame = ramdisk.setup_ring(res, sq, cq, notify_ep)?;
                    Ok(frame.cap())
                })
            },
            (_, _) => |_, _| {
                Err(Error::NotSupported)
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
