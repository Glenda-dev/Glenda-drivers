#![no_std]
#![no_main]

#[macro_use]
extern crate glenda;
extern crate alloc;

use glenda::cap::{CapType, ENDPOINT_SLOT, MONITOR_CAP};
use glenda::client::{FsClient, ProcessClient, ResourceClient};
use glenda::interface::{FileSystemService, ResourceService};
use glenda::ipc::Badge;
use glenda::protocol::fs::OpenFlags;

mod server;
use server::LoopBlockServer;

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("LoopDev");
    log!("Loop Device Driver starting...");

    let mut _proc_client = ProcessClient::new(MONITOR_CAP);
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // 1. Allocate Endpoint for Block Service
    if let Err(e) = res_client.alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT) {
        log!("Failed to allocate endpoint: {:?}", e);
        return 1;
    }

    // 2. Open backing file from VFS (Monitor)
    // We use temporary FsClient to open, then use the returned badge for LoopBlockServer
    let mut vfs_client = FsClient::new(MONITOR_CAP);
    log!("Opening /disk.img...");

    let file_badge = match vfs_client.open(Badge::null(), "/disk.img", OpenFlags::O_RDWR, 0) {
        Ok(b) => b,
        Err(e) => {
            log!("Failed to open /disk.img: {:?}", e);
            // Fallback for testing: maybe just assume 0 if open fails? No, return.
            return 1;
        }
    };
    log!("Opened /disk.img, handle: {}", file_badge);

    // 3. Create Server

    log!("Starting LoopBlockServer loop...");
    let mut server = LoopBlockServer::new(MONITOR_CAP, file_badge);

    if let Err(e) = server.run() {
        log!("LoopBlockServer failed: {:?}", e);
        return 1;
    }

    0
}
