use core::ptr::NonNull;
use glenda::cap::{Endpoint, Frame};
use glenda::io::uring::{self as io_uring, IoUringServer};
use glenda::mem::shm::SharedMemory;
use virtio_common::consts::*;
use virtio_common::queue::{Descriptor, VirtQueue, DESC_F_NEXT, DESC_F_WRITE};
use virtio_common::{Result, VirtIOError, VirtIOTransport};

pub const VIRTIO_NET_F_MAC: u64 = 5;
pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 15;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct VirtioNetHdr {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

pub struct VirtIONet {
    transport: VirtIOTransport,
    mac: [u8; 6],
    pub rx_queue: Option<VirtQueue>,
    pub tx_queue: Option<VirtQueue>,
    pub dma_vaddr: *mut u8,
    pub dma_paddr: u64,
    pub pending_rx: [Option<(u64, u16)>; 128],
    pub pending_tx: [Option<(u64, u16)>; 128],
    pub ring_server: Option<IoUringServer>,
    pub endpoint: Option<Endpoint>,
    pub buffer: Option<SharedMemory>,
}

impl VirtIONet {
    pub unsafe fn new(base_addr: usize) -> Result<Self> {
        let base = NonNull::new(base_addr as *mut u8).ok_or(VirtIOError::DeviceNotFound)?;
        let transport = VirtIOTransport::new(base)?;

        if transport.get_device_id() != DEV_ID_NET {
            log!("Unmatched device ID: {:#x}", transport.get_device_id());
            return Err(VirtIOError::DeviceNotFound);
        }

        Ok(Self {
            transport,
            mac: [0u8; 6],
            rx_queue: None,
            tx_queue: None,
            dma_vaddr: core::ptr::null_mut(),
            dma_paddr: 0,
            pending_rx: [None; 128],
            pending_tx: [None; 128],
            ring_server: None,
            endpoint: None,
            buffer: None,
        })
    }

    pub fn setup_shm(
        &mut self,
        frame: Frame,
        vaddr: usize,
        paddr: u64,
        size: usize,
    ) -> core::result::Result<(), glenda::error::Error> {
        let mut shm = SharedMemory::from_frame(frame, vaddr, size);
        shm.set_client_vaddr(vaddr);
        shm.set_paddr(paddr);
        self.buffer = Some(shm);
        log!("SHM setup: client_vaddr={:#x}, paddr={:#x}, size={}", vaddr, paddr, size);
        Ok(())
    }

    pub fn init(&mut self, dma_vaddr: *mut u8, dma_paddr: u64, endpoint: Endpoint) -> Result<()> {
        self.dma_vaddr = dma_vaddr;
        self.dma_paddr = dma_paddr;
        self.endpoint = Some(endpoint);
        self.transport.set_status(0);
        self.transport.add_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        let mut device_features = self.transport.get_features();
        device_features &= !(1 << VIRTIO_NET_F_MRG_RXBUF);
        self.transport.set_features(device_features);

        self.transport.add_status(STATUS_FEATURES_OK);
        if (self.transport.get_status() & STATUS_FEATURES_OK) == 0 {
            return Err(VirtIOError::InvalidHeader);
        }

        // Use Page 2+ for VirtQueues
        let rx_paddr = dma_paddr + 8192;
        let rx_vaddr = unsafe { dma_vaddr.add(8192) };
        let rx_queue = unsafe { VirtQueue::new(0, 128, rx_paddr, rx_vaddr) };
        unsafe { self.transport.setup_queue(&rx_queue) };
        self.rx_queue = Some(rx_queue);

        let tx_paddr = rx_paddr + 4096;
        let tx_vaddr = unsafe { rx_vaddr.add(4096) };
        let tx_queue = unsafe { VirtQueue::new(1, 128, tx_paddr, tx_vaddr) };
        unsafe { self.transport.setup_queue(&tx_queue) };
        self.tx_queue = Some(tx_queue);

        let mac_ptr = unsafe { self.transport.config_ptr() };
        for i in 0..6 {
            self.mac[i] = unsafe { core::ptr::read_volatile(mac_ptr.add(i)) };
        }
        self.transport.add_status(STATUS_DRIVER_OK);
        Ok(())
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
    pub fn set_ring_server(&mut self, server: IoUringServer) {
        self.ring_server = Some(server);
    }
    pub fn set_endpoint(&mut self, endpoint: Endpoint) {
        self.endpoint = Some(endpoint);
    }

    pub fn handle_ring(&mut self) {
        let mut sqes = [io_uring::IoUringSqe::default(); 16];
        let mut count = 0;

        if let Some(server) = self.ring_server.as_mut() {
            while count < 16 {
                if let Some(sqe) = server.next_request() {
                    sqes[count] = sqe;
                    count += 1;
                } else {
                    break;
                }
            }
        } else {
            return;
        }

        for i in 0..count {
            let sqe = sqes[i];
            let res = match sqe.opcode {
                io_uring::IOURING_OP_READ => self.submit(0, sqe),
                io_uring::IOURING_OP_WRITE => self.submit(1, sqe),
                _ => Err(VirtIOError::DeviceNotFound),
            };
            if res.is_err() {
                if let Some(server) = self.ring_server.as_mut() {
                    let _ = server.complete(sqe.user_data, -1);
                }
            }
        }
    }

    fn submit(&mut self, qidx: u32, sqe: io_uring::IoUringSqe) -> Result<()> {
        let queue = if qidx == 0 { self.rx_queue.as_mut() } else { self.tx_queue.as_mut() }
            .ok_or(VirtIOError::DeviceNotFound)?;

        let d1 = queue.alloc_desc().ok_or(VirtIOError::OOM)?;
        let d2 = queue.alloc_desc().ok_or(VirtIOError::OOM)?;

        let data_paddr = if let Some(ref shm) = self.buffer {
            let client_vaddr = shm.client_vaddr();
            let paddr = shm.paddr();
            let size = shm.size();
            if (sqe.addr as usize) < client_vaddr
                || (sqe.addr as usize) + sqe.len as usize > client_vaddr + size
            {
                error!("Address {:#x} out of SHM boundary", sqe.addr);
                return Err(VirtIOError::InvalidHeader);
            }
            paddr + (sqe.addr as u64 - client_vaddr as u64)
        } else {
            sqe.addr
        };

        // Page 0 (DMA_VA) for RX headers, Page 1 for TX headers.
        // Each page can hold up to 128 headers (approx 12 bytes each).
        let hdr_paddr = self.dma_paddr + (qidx as u64 * 4096) + (d1 as u64 * 16);
        let hdr_vaddr = unsafe { self.dma_vaddr.add(qidx as usize * 4096).add(d1 as usize * 16) };

        // Initialize header
        unsafe {
            let hdr = hdr_vaddr as *mut VirtioNetHdr;
            hdr.write_volatile(VirtioNetHdr::default());
        }

        // Desc 1: Header
        queue.write_desc(
            d1,
            Descriptor {
                addr: hdr_paddr,
                len: core::mem::size_of::<VirtioNetHdr>() as u32,
                flags: if qidx == 0 { DESC_F_NEXT | DESC_F_WRITE } else { DESC_F_NEXT },
                next: d2,
            },
        );

        // Desc 2: Payload
        queue.write_desc(
            d2,
            Descriptor {
                addr: data_paddr,
                len: sqe.len,
                flags: if qidx == 0 { DESC_F_WRITE } else { 0 },
                next: 0,
            },
        );

        if qidx == 0 {
            self.pending_rx[d1 as usize] = Some((sqe.user_data, d1));
        } else {
            self.pending_tx[d1 as usize] = Some((sqe.user_data, d1));
        }

        glenda::arch::sync::fence();
        queue.submit(d1);
        self.transport.notify(qidx);
        Ok(())
    }

    pub fn handle_irq(&mut self) {
        if !self.transport.interrupt_ack() {
            return;
        }

        let server = match self.ring_server.as_mut() {
            Some(s) => s,
            None => return,
        };

        if let Some(rx) = self.rx_queue.as_mut() {
            while let Some((idx, len)) = rx.pop() {
                if let Some((data, head)) = self.pending_rx[idx as usize].take() {
                    // len includes the header size in mergeable rx buffer or similar?
                    // Actually, virtio-net-hdr is part of the chain length.
                    let result_len = if len as usize > core::mem::size_of::<VirtioNetHdr>() {
                        len - core::mem::size_of::<VirtioNetHdr>() as u32
                    } else {
                        0
                    };
                    let _ = server.complete(data, result_len as i32);

                    // Free chains
                    let mut curr = head;
                    loop {
                        let row = &rx.desc_table()[curr as usize];
                        let flags = row.flags;
                        let next = row.next;
                        rx.free_desc(curr);
                        if flags & DESC_F_NEXT == 0 {
                            break;
                        }
                        curr = next;
                    }
                }
            }
        }

        if let Some(tx) = self.tx_queue.as_mut() {
            while let Some((idx, _)) = tx.pop() {
                if let Some((data, head)) = self.pending_tx[idx as usize].take() {
                    let _ = server.complete(data, 0);

                    // Free chains
                    let mut curr = head;
                    loop {
                        let row = &tx.desc_table()[curr as usize];
                        let flags = row.flags;
                        let next = row.next;
                        tx.free_desc(curr);
                        if flags & DESC_F_NEXT == 0 {
                            break;
                        }
                        curr = next;
                    }
                }
            }
        }
    }
}
