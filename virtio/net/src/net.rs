use core::ptr::NonNull;
use virtio_common::consts::*;
use virtio_common::{Result, VirtIOError, VirtIOTransport};

pub struct VirtIONet {
    transport: VirtIOTransport,
    mac: [u8; 6],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtIONetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
    // num_buffers is only available if VIRTIO_NET_F_MRG_RXBUF is negotiated.
    // For now we assume simplistic header.
}

impl VirtIONet {
    pub unsafe fn new(base_addr: usize) -> Result<Self> {
        let base = NonNull::new(base_addr as *mut u8).ok_or(VirtIOError::DeviceNotFound)?;
        let transport = VirtIOTransport::new(base)?;

        if transport.get_device_id() != DEV_ID_NET {
            return Err(VirtIOError::DeviceNotFound);
        }

        // Reset
        transport.set_status(0);

        // Acknowledge
        let mut status = transport.get_status();
        status |= 1; // ACKNOWLEDGE
        transport.set_status(status);

        status |= 2; // DRIVER
        transport.set_status(status);

        // Feature negotiation (todo: mac, etc)
        // For now, accept what device offers (simple) or set basic ones.
        // Let's assume we want MAC.
        // VIRTIO_NET_F_MAC = 5 (1 << 5)
        let device_features = transport.get_features();
        let driver_features = device_features & (1 << 5);
        transport.set_features(driver_features);

        status |= 8; // FEATURES_OK
        transport.set_status(status);

        // Check if FEATURES_OK is still set
        if (transport.get_status() & 8) == 0 {
            return Err(VirtIOError::InvalidHeader); // Feature negotiation failed
        }

        // Initialize Queues (Stub)
        // virtio-common does not currently provide VirtQueue struct logic.
        // real implementation would init RX/TX queues here.
        // for now we just proceed to DRIVER_OK.

        // Read MAC
        // Only if VIRTIO_NET_F_MAC negotiated.
        // Config space is after MMIO regs (0x100) usually ?
        // Transport `read_config` needed?
        // Let's assume generic transport exposes way to read config.
        // Or implement specific config read here.
        // Virtio 1.0 MMIO: config is at 0x100.
        // Net config:
        // struct virtio_net_config {
        //   u8 mac[6];
        //   u16 status;
        //   u16 max_virtqueue_pairs;
        //   u16 mtu;
        // }
        // Offset 0x100.
        let mac_ptr = transport.config_ptr();
        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = core::ptr::read_volatile(mac_ptr.add(i));
        }

        log::info!(
            "VirtIO Net MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );

        status |= 4; // DRIVER_OK
        transport.set_status(status);

        Ok(Self { transport, mac })
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
}
