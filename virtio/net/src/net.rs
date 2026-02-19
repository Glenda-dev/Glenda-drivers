use crate::log;
use core::ptr::NonNull;
use glenda::cap::Endpoint;
use glenda::mem::io_uring;
use glenda_drivers::io_uring::IoRingServer;
use virtio_common::consts::*;
use virtio_common::queue::{VirtQueue, Descriptor, DESC_F_WRITE};
use virtio_common::{Result, VirtIOError, VirtIOTransport};

pub const VIRTIO_NET_F_MAC: u64 = 5;
pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 15;

pub struct VirtIONet {
    transport: VirtIOTransport,
    mac: [u8; 6],
    pub rx_queue: Option<VirtQueue>,
    pub tx_queue: Option<VirtQueue>,
    pub pending_rx: [Option<(u64, u16)>; 128],
    pub pending_tx: [Option<(u64, u16)>; 128],
    pub ring_server: Option<IoRingServer>,
    pub endpoint: Option<Endpoint>,
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
            pending_rx: [None; 128],
            pending_tx: [None; 128],
            ring_server: None,
            endpoint: None,
        })
    }
    
    pub fn init(&mut self, dma_vaddr: *mut u8, dma_paddr: u64, endpoint: Endpoint) -> Result<()> {
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

        let rx_queue = unsafe { VirtQueue::new(0, 128, dma_paddr, dma_vaddr) };
        unsafe { self.transport.setup_queue(&rx_queue) };
        self.rx_queue = Some(rx_queue);
        
        let tx_paddr = dma_paddr + 4096;
        let tx_vaddr = unsafe { dma_vaddr.add(4096) };
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

    pub fn mac(&self) -> [u8; 6] { self.mac }
    pub fn set_ring_server(&mut self, server: IoRingServer) { self.ring_server = Some(server); }
    pub fn set_endpoint(&mut self, endpoint: Endpoint) { self.endpoint = Some(endpoint); }

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
                io_uring::IORING_OP_READ => self.submit(0, sqe),
                io_uring::IORING_OP_WRITE => self.submit(1, sqe),
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
        let queue = if qidx == 0 { self.rx_queue.as_mut() } else { self.tx_queue.as_mut() }.ok_or(VirtIOError::DeviceNotFound)?;
        let head = queue.alloc_desc().ok_or(VirtIOError::OOM)?;
        let descs = queue.desc_table();
        
        descs[head as usize] = Descriptor {
            addr: sqe.addr as u64,
            len: sqe.len,
            flags: if qidx == 0 { DESC_F_WRITE } else { 0 },
            next: 0,
        };
        
        if qidx == 0 {
            self.pending_rx[head as usize] = Some((sqe.user_data, head));
        } else {
            self.pending_tx[head as usize] = Some((sqe.user_data, head));
        }
        
        queue.submit(head);
        unsafe { self.transport.notify(qidx) };
        Ok(())
    }
    
    pub fn handle_irq(&mut self) {
        let mut _needs_notify = false;
        let server = match self.ring_server.as_mut() {
            Some(s) => s,
            None => return,
        };

        if let Some(rx) = self.rx_queue.as_mut() {
            while let Some((idx, len)) = rx.pop() {
                if let Some((data, _)) = self.pending_rx[idx as usize].take() {
                    let _ = server.complete(data, len as i32);
                    _needs_notify = true;
                }
                rx.free_desc(idx as u16);
            }
        }
        
        if let Some(tx) = self.tx_queue.as_mut() {
            while let Some((idx, _)) = tx.pop() {
                if let Some((data, _)) = self.pending_tx[idx as usize].take() {
                    let _ = server.complete(data, 0);
                    _needs_notify = true;
                }
                tx.free_desc(idx as u16);
            }
        }
    }
}
