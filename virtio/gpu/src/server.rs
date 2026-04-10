use crate::gpu::VirtIOGpu;
use crate::layout::{
    DMA_SLOT, DMA_VA, IRQ_BADGE, IRQ_NOTIFY_CAP, IRQ_NOTIFY_SLOT, IRQ_SLOT, MMIO_SLOT, MMIO_VA,
    RING_SLOT, RING_VA,
};
use alloc::string::String;
use core::ptr::NonNull;
use glenda::cap::{CapPtr, CapType, Endpoint, Frame, IrqHandler, Reply, Rights, CSPACE_CAP};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::drivers::protocol::{fb, FB_PROTO};
use glenda::error::Error;
use glenda::interface::{
    CSpaceService, DeviceService, ResourceService, SystemService, VSpaceService,
};
use glenda::io::uring::{IoUringBuffer as IoUring, IoUringServer};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::protocol::device::LogicDeviceDesc;
use glenda::utils::manager::{CSpaceManager, VSpaceManager};
use virtio_common::VirtIOTransport;

pub struct GpuService<'a> {
    pub dev: &'a mut DeviceClient,
    pub res: &'a mut ResourceClient,
    pub cspace_mgr: &'a mut CSpaceManager,
    pub vspace_mgr: &'a mut VSpaceManager,
    pub endpoint: Endpoint,
    pub reply: CapPtr,
    pub recv: CapPtr,
    pub gpu: Option<VirtIOGpu>,
    pub irq: Option<IrqHandler>,
    pub fb_info: fb::FbInfo,
}

impl<'a> GpuService<'a> {
    pub fn new(
        dev: &'a mut DeviceClient,
        res: &'a mut ResourceClient,
        cspace_mgr: &'a mut CSpaceManager,
        vspace_mgr: &'a mut VSpaceManager,
        endpoint: Endpoint,
        reply: CapPtr,
    ) -> Self {
        Self {
            dev,
            res,
            cspace_mgr,
            vspace_mgr,
            endpoint,
            reply,
            recv: CapPtr::null(),
            gpu: None,
            irq: None,
            fb_info: fb::FbInfo::default(),
        }
    }

    pub fn setup_ring(
        &mut self,
        sq_entries: u32,
        cq_entries: u32,
        notify_ep: Endpoint,
        _recv: CapPtr,
    ) -> Result<Frame, Error> {
        // Use RING_SLOT instead of allocating a temporary one if possible
        let slot = RING_SLOT;
        let frame_cap = self.res.alloc(Badge::null(), CapType::Frame, 1, slot)?;
        let frame = Frame::from(frame_cap);

        self.vspace_mgr.map_frame(
            frame.clone(),
            RING_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace_mgr,
        )?;
        glenda::arch::sync::fence();

        let ring = unsafe {
            IoUring::new(RING_VA as *mut u8, glenda::arch::mem::PGSIZE, sq_entries, cq_entries)
        };
        let mut server = IoUringServer::new(ring);
        server.set_client_notify(notify_ep);

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_ring_server(server);
        }

        Ok(frame)
    }
}

impl<'a> SystemService for GpuService<'a> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Driver init...");
        // 1. Get and map MMIO
        let (mmio, _pa, _size) = self.dev.get_mmio(Badge::null(), 0, MMIO_SLOT)?;
        self.vspace_mgr.map_frame(
            mmio,
            MMIO_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            1,
            self.res,
            self.cspace_mgr,
        )?;
        glenda::arch::sync::fence();

        // 2. Get and setup IRQ
        let irq_handler = self.dev.get_irq(Badge::null(), 0, IRQ_SLOT)?;
        CSPACE_CAP.mint_self(
            self.endpoint.cap(),
            IRQ_NOTIFY_SLOT,
            Badge::new(IRQ_BADGE),
            Rights::ALL,
        )?;
        irq_handler.set_notification(IRQ_NOTIFY_CAP)?;
        self.irq = Some(irq_handler);

        // 3. Setup VirtIO transport
        let transport = unsafe {
            VirtIOTransport::new(NonNull::new(MMIO_VA as *mut u8).expect("MMIO_VA is null"))
                .expect("Failed to init transport")
        };

        // 4. Allocate and map DMA memory for command buffers and queues
        let (paddr, frame) = self.res.dma_alloc(Badge::null(), 4, DMA_SLOT)?;
        self.vspace_mgr.map_frame(
            frame,
            DMA_VA,
            glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
            4,
            self.res,
            self.cspace_mgr,
        )?;
        glenda::arch::sync::fence();

        let mut gpu = VirtIOGpu::new(transport, DMA_VA as *mut u8, paddr as usize);

        // 5. Initialize GPU hardware
        gpu.init()?;
        glenda::arch::sync::fence();

        self.fb_info.width = gpu.width();
        self.fb_info.height = gpu.height();
        self.fb_info.pitch = gpu.width() * 4;
        self.fb_info.bpp = 32;
        self.fb_info.size = gpu.width() * gpu.height() * 4;
        self.fb_info.format = glenda::drivers::protocol::fb::FB_FORMAT_XRGB8888;
        self.gpu = Some(gpu);

        let desc = LogicDeviceDesc {
            name: String::from("virtio-gpu"),
            parent_name: String::from("root"),
            dev_type: glenda::protocol::device::LogicDeviceType::Fb,
            badge: None,
        };
        self.dev.register_logic(Badge::null(), desc, self.endpoint.cap())?;

        log!("Initialized: {}x{}", self.fb_info.width, self.fb_info.height);
        Ok(())
    }

    fn listen(&mut self, endpoint: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.endpoint = endpoint;
        self.reply = reply;
        self.recv = recv;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.reply);
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
                    "Failed to dispatch message for {:#x}: {:?}, proto={:#x}, label={:#x}",
                    badge.bits(),
                    e,
                    proto,
                    label
                );
                utcb.set_msg_tag(MsgTag::err());
                utcb.set_mr(0, e as usize);
            }
            if let Err(e) = self.reply(&mut utcb) {
                error!("Reply failed: {:?}", e);
            }
        }
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        glenda::ipc_dispatch! {
            self, utcb,
            (glenda::protocol::KERNEL_PROTO, glenda::protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |u| {
                    let badge = u.get_badge().bits();
                    let is_ring = badge & (glenda::io::uring::NOTIFY_IO_URING_CQ | glenda::io::uring::NOTIFY_IO_URING_SQ) != 0;
                    let is_irq = badge & IRQ_BADGE != 0;

                    if let Some(gpu) = s.gpu.as_mut() {
                        if is_irq {
                            let _ = gpu.ack_interrupt();
                            if let Some(irq) = s.irq.as_ref() {
                                let _ = irq.ack();
                            }
                        }
                        if is_ring {
                            if let Err(e) = gpu.handle_ring() {
                                error!("Failed to handle ring: {:?}", e);
                            }
                        }
                    }
                    Ok(())
                })
            },
            (FB_PROTO, fb::GET_INFO) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    unsafe { u.write_obj(&s.fb_info)?; }
                    Ok(0usize)
                })
            },
            (FB_PROTO, fb::FLUSH) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let x = u.get_mr(0);
                    let y = u.get_mr(1);
                    let w = u.get_mr(2);
                    let h = u.get_mr(3);
                    if let Some(gpu) = s.gpu.as_mut() {
                        gpu.flush(x, y, w, h)?;
                    }
                    Ok(0usize)
                })
            },
            (FB_PROTO, fb::SET_SCANOUT) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let paddr = u.get_mr(0);
                    if let Some(gpu) = s.gpu.as_mut() {
                        gpu.set_scanout(paddr)?;
                    }
                    Ok(0usize)
                })
            },
            (FB_PROTO, fb::SETUP_BUFFER) => |s: &mut Self, u: &mut UTCB| {
                handle_cap_call(u, |u| {
                    let size = s.fb_info.size;
                    let pages = glenda::utils::align::align_up(size, glenda::arch::mem::PGSIZE) / glenda::arch::mem::PGSIZE;
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    let (paddr, frame) = s.res.dma_alloc(glenda::ipc::Badge::null(), pages, slot)?;

                    log!("SHM allocated, paddr 0x{:x}, size {}", paddr, size);
                    s.fb_info.paddr = paddr;

                    u.set_mr(0, paddr);
                    u.set_mr(1, size);

                    Ok(frame.cap())
                })
            },
            (FB_PROTO, fb::SETUP_RING) => |s: &mut Self, u: &mut UTCB| {
                handle_cap_call(u, |u| {
                    let recv_slot = s.recv;
                    let slot = s.cspace_mgr.alloc(s.res)?;
                    let sq_entries = u.get_mr(0) as u32;
                    let cq_entries = u.get_mr(1) as u32;

                    CSPACE_CAP.transfer_self(recv_slot, slot)?;
                    let notify_ep = Endpoint::from(slot);

                    let frame = s.setup_ring(sq_entries, cq_entries, notify_ep, CapPtr::null())?;
                    Ok(frame.cap())
                })
            }
        }
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        Reply::from(self.reply).reply(utcb)
    }

    fn stop(&mut self) {}
}
