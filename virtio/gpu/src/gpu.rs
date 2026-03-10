use crate::protocol::*;
use glenda::drivers::protocol::fb::IOURING_OP_FB_FLUSH;
use glenda::error::Error;
use glenda::io::uring::IoUringServer;
use virtio_common::consts::*;
use virtio_common::{Descriptor, VirtIOTransport, VirtQueue, DESC_F_NEXT, DESC_F_WRITE};
pub struct VirtIOGpu {
    transport: VirtIOTransport,
    width: usize,
    height: usize,
    ring_server: Option<IoUringServer>,
    control_vq: Option<VirtQueue>,
    cursor_vq: Option<VirtQueue>,
    cmd_buf_pa: usize,
    cmd_buf_va: *mut u8,
}

impl VirtIOGpu {
    pub fn new(transport: VirtIOTransport, cmd_buf_va: *mut u8, cmd_buf_pa: usize) -> Self {
        Self {
            transport,
            width: 0,
            height: 0,
            ring_server: None,
            control_vq: None,
            cursor_vq: None,
            cmd_buf_va,
            cmd_buf_pa,
        }
    }

    pub fn set_ring_server(&mut self, server: IoUringServer) {
        self.ring_server = Some(server);
    }

    pub fn handle_ring(&mut self) -> Result<(), Error> {
        if let Some(mut server) = self.ring_server.take() {
            while let Some(sqe) = server.next_request() {
                if sqe.opcode == IOURING_OP_FB_FLUSH {
                    let x = (sqe.off >> 32) as usize;
                    let y = (sqe.off & 0xFFFFFFFF) as usize;
                    let w = (sqe.addr >> 32) as usize;
                    let h = (sqe.addr & 0xFFFFFFFF) as usize;
                    let res = self.flush(x, y, w, h);
                    let ret = if res.is_ok() { 0 } else { -1 };
                    server.complete(sqe.user_data, ret as i32)?;
                } else {
                    error!("Unknown IOUring opcode: {}", sqe.opcode);
                    server.complete(sqe.user_data, -1)?;
                }
            }
            self.ring_server = Some(server);
        } else {
            warn!("Ring server not set, cannot handle ring events");
        }
        Ok(())
    }

    pub fn init(&mut self) -> Result<(), Error> {
        // Use transport to reset and identify the device
        self.transport.set_status(0);
        self.transport.add_status(STATUS_ACKNOWLEDGE);
        self.transport.add_status(STATUS_DRIVER);

        // 1. Feature negotiation
        let features = self.transport.get_device_features();
        // We only need basic VirtIO 1.0 features
        self.transport.set_driver_features(features & (1 << 32)); // VIRTIO_F_VERSION_1
        self.transport.add_status(STATUS_FEATURES_OK);
        if (self.transport.get_status() & STATUS_FEATURES_OK) == 0 {
            return Err(Error::NotSupported);
        }

        // 2. Setup VirtQueues (0: controlvq, 1: cursorvq)
        // We use the cmd_buf to host queues.
        // Queue size 16 * 16 (desc) + 6 + 32 (avail) + 6 + 128 (used) = ~200 bytes.
        // We can place them at the start of cmd_buf.
        unsafe {
            let vq0 = VirtQueue::new(0, 16, self.cmd_buf_pa, self.cmd_buf_va);
            self.transport.setup_queue(&vq0);
            self.control_vq = Some(vq0);

            let q1_offset = 4096; // Offset for cursor vq
            let vq1 =
                VirtQueue::new(1, 16, self.cmd_buf_pa + q1_offset, self.cmd_buf_va.add(q1_offset));
            self.transport.setup_queue(&vq1);
            self.cursor_vq = Some(vq1);
        }

        self.transport.add_status(STATUS_DRIVER_OK);

        // Get display info to set width/height
        let display_cmd = GpuHeader { ty: GpuCmdType::GetDisplayInfo as u32, ..Default::default() };
        let info: GpuDisplayInfo = self.send_cmd(display_cmd)?;

        // Use the first enabled display mode, default to 1280x720 if none or error
        if info.hdr.ty == GpuCmdType::RespOkDisplayInfo as u32 {
            self.width = info.pmodes[0].r.width as usize;
            self.height = info.pmodes[0].r.height as usize;
            log!("Detected resolution {}x{}", self.width, self.height);
        } else {
            warn!("VirtIO-GPU: Failed to get display info, using default 1280x720");
            self.width = 1280;
            self.height = 720;
        }

        Ok(())
    }

    fn send_cmd<T, R>(&mut self, cmd: T) -> Result<R, Error>
    where
        T: Copy,
        R: Copy + Default,
    {
        let vq = self.control_vq.as_mut().ok_or(Error::NotInitialized)?;

        // Use cmd_buf offset 8192 for command data and 12288 for response
        let cmd_offset = 8192;
        let resp_offset = 12288;

        unsafe {
            core::ptr::write_volatile(self.cmd_buf_va.add(cmd_offset) as *mut T, cmd);
            core::ptr::write_volatile(self.cmd_buf_va.add(resp_offset) as *mut R, R::default());
        }

        let head = vq.alloc_desc().ok_or(Error::OutOfMemory)?;
        let resp_desc_id = vq.alloc_desc().ok_or(Error::OutOfMemory)?;

        vq.write_desc(
            head,
            Descriptor {
                addr: self.cmd_buf_pa + cmd_offset,
                len: core::mem::size_of::<T>() as u32,
                flags: DESC_F_NEXT,
                next: resp_desc_id,
            },
        );

        vq.write_desc(
            resp_desc_id,
            Descriptor {
                addr: self.cmd_buf_pa + resp_offset,
                len: core::mem::size_of::<R>() as u32,
                flags: DESC_F_WRITE,
                next: 0,
            },
        );

        vq.submit(head);
        self.transport.notify(0);

        while !vq.can_pop() {
            core::hint::spin_loop();
        }

        vq.pop();
        vq.free_desc(head);
        vq.free_desc(resp_desc_id);

        let resp =
            unsafe { core::ptr::read_volatile(self.cmd_buf_va.add(resp_offset) as *const R) };
        Ok(resp)
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn ack_interrupt(&self) -> u32 {
        self.transport.ack_interrupt()
    }

    pub fn flush(&mut self, x: usize, y: usize, w: usize, h: usize) -> Result<(), Error> {
        let res_id = 1;

        let transfer_cmd = GpuTransferToHost2d {
            hdr: GpuHeader { ty: GpuCmdType::TransferToHost2d as u32, ..Default::default() },
            r: GpuRect { x: x as u32, y: y as u32, width: w as u32, height: h as u32 },
            offset: ((y * self.width + x) * 4) as u64,
            resource_id: res_id,
            padding: 0,
        };
        let _resp: GpuHeader = self.send_cmd(transfer_cmd)?;

        let flush_cmd = GpuResourceFlush {
            hdr: GpuHeader { ty: GpuCmdType::ResourceFlush as u32, ..Default::default() },
            r: GpuRect { x: x as u32, y: y as u32, width: w as u32, height: h as u32 },
            resource_id: res_id,
            padding: 0,
        };
        let _resp: GpuHeader = self.send_cmd(flush_cmd)?;
        Ok(())
    }

    pub fn set_scanout(&mut self, paddr: usize) -> Result<(), Error> {
        log!("Setting scanout to paddr: 0x{:x}", paddr);

        let res_id = 1;
        // 1. Resource Create 2D
        let create_cmd = GpuResourceCreate2d {
            hdr: GpuHeader { ty: GpuCmdType::ResourceCreate2d as u32, ..Default::default() },
            resource_id: res_id,
            format: GpuFormats::B8G8R8A8Unorm as u32,
            width: self.width as u32,
            height: self.height as u32,
        };
        let resp: GpuHeader = self.send_cmd(create_cmd)?;
        if resp.ty != GpuCmdType::RespOkNoData as u32 {
            return Err(Error::IoError);
        }

        // 2. Resource Attach Backing
        // We need to send the GpuResourceAttachBacking command followed by a GpuMemEntry.
        // For simplicity in this single-descriptor implementation, we use a fixed structure if possible,
        // or just send the attachment.
        // Note: The spec says nr_entries follows the header, then an array of GpuMemEntry.
        #[repr(C)]
        #[derive(Copy, Clone)]
        struct AttachWithEntry {
            cmd: GpuResourceAttachBacking,
            entry: GpuMemEntry,
        }

        let attach_cmd = AttachWithEntry {
            cmd: GpuResourceAttachBacking {
                hdr: GpuHeader {
                    ty: GpuCmdType::ResourceAttachBacking as u32,
                    ..Default::default()
                },
                resource_id: res_id,
                nr_entries: 1,
            },
            entry: GpuMemEntry {
                addr: paddr as u64,
                length: (self.width * self.height * 4) as u32,
                padding: 0,
            },
        };
        let resp: GpuHeader = self.send_cmd(attach_cmd)?;
        if resp.ty != GpuCmdType::RespOkNoData as u32 {
            return Err(Error::IoError);
        }

        // 3. Set Scanout
        let scanout_cmd = GpuSetScanout {
            hdr: GpuHeader { ty: GpuCmdType::SetScanout as u32, ..Default::default() },
            r: GpuRect { x: 0, y: 0, width: self.width as u32, height: self.height as u32 },
            scanout_id: 0,
            resource_id: res_id,
        };
        let resp: GpuHeader = self.send_cmd(scanout_cmd)?;
        if resp.ty != GpuCmdType::RespOkNoData as u32 {
            return Err(Error::IoError);
        }

        // 4. Resource Flush (Initial)
        let transfer_cmd = GpuTransferToHost2d {
            hdr: GpuHeader { ty: GpuCmdType::TransferToHost2d as u32, ..Default::default() },
            r: GpuRect { x: 0, y: 0, width: self.width as u32, height: self.height as u32 },
            offset: 0,
            resource_id: res_id,
            padding: 0,
        };
        let _resp: GpuHeader = self.send_cmd(transfer_cmd)?;

        let flush_cmd = GpuResourceFlush {
            hdr: GpuHeader { ty: GpuCmdType::ResourceFlush as u32, ..Default::default() },
            r: GpuRect { x: 0, y: 0, width: self.width as u32, height: self.height as u32 },
            resource_id: res_id,
            padding: 0,
        };
        let _resp: GpuHeader = self.send_cmd(flush_cmd)?;

        Ok(())
    }
}
