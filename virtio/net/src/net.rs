use crate::log;
use core::ptr::NonNull;
use virtio_common::consts::*;
use virtio_common::{Result, VirtIOError, VirtIOTransport};

pub struct VirtIONet {
    _transport: VirtIOTransport,
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
            log!("Unmatched device ID: {:#x}", transport.get_device_id());
            return Err(VirtIOError::DeviceNotFound);
        }

        // Reset
        transport.set_status(0);

        // Acknowledge
        transport.add_status(STATUS_ACKNOWLEDGE);
        transport.add_status(STATUS_DRIVER);

        // Feature negotiation
        let device_features = transport.get_features();
        log!("Device features: {:#x}", device_features);
        // Accept all features for now
        transport.set_features(device_features);

        transport.add_status(STATUS_FEATURES_OK);
        // Check if FEATURES_OK is still set
        if (transport.get_status() & STATUS_FEATURES_OK) == 0 {
            log!("Feature negotiation failed, status: {:#x}", transport.get_status());
            return Err(VirtIOError::InvalidHeader);
        }

        // Read MAC
        let mac_ptr = transport.config_ptr();
        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = core::ptr::read_volatile(mac_ptr.add(i));
        }
        log!(
            "MAC Address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );

        transport.add_status(STATUS_DRIVER_OK);
        Ok(Self { _transport: transport, mac })
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
}
