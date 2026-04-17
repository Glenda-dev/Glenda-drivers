mod config;
mod consts;

use crate::layout::SHM_VA;
use alloc::collections::VecDeque;
use config::*;
use consts::*;
use glenda::cap::{IrqHandler, Page};
use glenda::drivers::interface::UartDriver;
use glenda::error::Error;
use glenda::io::ring_buffer::ShmRingBuffer;
use glenda::io::uring::IoUringServer;
use glenda::mem::shm::SharedMemory;

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    pub ring: Option<IoUringServer>,
    pub shm: Option<SharedMemory>,
    pub rx_ring: Option<&'static mut ShmRingBuffer>,
    pub tx_ring: Option<&'static mut ShmRingBuffer>,
    pub rx_buffer: VecDeque<u8>,
    pub pending_read: Option<usize>,
}

impl Ns16550a {
    pub fn new(base: usize, irq: IrqHandler) -> Self {
        Self {
            base,
            irq,
            ring: None,
            shm: None,
            rx_ring: None,
            tx_ring: None,
            rx_buffer: VecDeque::with_capacity(1024),
            pending_read: None,
        }
    }

    pub fn set_ring_server(&mut self, ring: IoUringServer) {
        self.ring = Some(ring);
    }

    pub fn setup_shm(
        &mut self,
        frame: Page,
        client_vaddr: usize,
        paddr: usize,
        size: usize,
    ) -> Result<(), Error> {
        let mut shm = SharedMemory::new(frame, SHM_VA, size);
        shm.set_client_vaddr(client_vaddr);
        shm.set_paddr(paddr);

        // Split SHM into two halves
        // 0 - 2KB: TX Ring Buffer (Input to UART)
        // 2KB - 4KB: RX Ring Buffer (Output from UART)

        let tx_ring_ptr = SHM_VA as *mut u8;
        let rx_ring_ptr = (SHM_VA + 2048) as *mut u8;

        unsafe {
            self.tx_ring = Some(ShmRingBuffer::init(tx_ring_ptr, 2048));
            self.rx_ring = Some(ShmRingBuffer::init(rx_ring_ptr, 2048));
        }

        self.shm = Some(shm);
        Ok(())
    }

    pub unsafe fn read_reg(&self, offset: usize) -> u8 {
        let ptr = (self.base + offset) as *const u8;
        core::ptr::read_volatile(ptr)
    }

    pub unsafe fn write_reg(&self, offset: usize, val: u8) {
        let ptr = (self.base + offset) as *mut u8;
        core::ptr::write_volatile(ptr, val);
    }

    pub fn init_hw(&self) {
        self.set_baud_rate(DEFAULT_BAUD_RATE);
        unsafe {
            // Enable RX interrupt
            self.write_reg(IER, IER_RX_ENABLE);
        }
    }

    pub fn set_baud_rate(&self, baud: u32) {
        let divisor = calculate_divisor(baud);
        unsafe {
            // Disable interrupts during init
            self.write_reg(IER, 0x00);

            // Enable DLAB to set baud rate
            self.write_reg(LCR, LCR_DLAB);

            // Set divisor
            self.write_reg(DLL, (divisor & 0xFF) as u8);
            self.write_reg(DLM, (divisor >> 8) as u8);

            // 8 bits, no parity, one stop bit (8N1), disable DLAB
            self.write_reg(LCR, LCR_DATA_BITS_8 | LCR_STOP_BITS_1 | LCR_PARITY_NONE);

            // Enable FIFO, clear them, with 14-byte threshold
            self.write_reg(FCR, FCR_FIFO_ENABLE | FCR_FIFO_RX_RESET | FCR_FIFO_TX_RESET);

            // IRQs enabled, RTS/DTR set
            self.write_reg(MCR, MCR_OUT2 | MCR_RTS | MCR_DTR);
        }
    }

    pub fn handle_irq(&mut self) -> Result<(), Error> {
        let mut has_data = false;
        loop {
            unsafe {
                let iir = self.read_reg(IIR);
                if iir & IIR_NO_INTERRUPT != 0 {
                    break;
                }

                let id = iir & IIR_ID_MASK;
                if id == IIR_RX_DATA_READY || id == IIR_TIMEOUT || id == IIR_RLS {
                    while let Some(c) = self.getchar() {
                        if self.rx_buffer.len() < 1024 {
                            self.rx_buffer.push_back(c);
                            has_data = true;
                        } else {
                            warn!("NS16550A: RX buffer overflow, dropping byte {:#x}", c);
                        }
                    }
                } else if id == IIR_THR_EMPTY {
                    self.process_tx_ring();
                }
            }
        }

        if has_data {
            self.process_rx_ring();
        }
        Ok(())
    }

    fn process_rx_ring(&mut self) {
        // Try to push to SHM Ring Buffer first
        if let Some(ring) = &mut self.rx_ring {
            let mut pushed_total = 0;
            while !self.rx_buffer.is_empty() {
                let first = *self.rx_buffer.front().unwrap();
                if ring.push_byte(first) {
                    self.rx_buffer.pop_front();
                    pushed_total += 1;
                } else {
                    break; // Ring full
                }
            }

            if pushed_total > 0 {
                if let Some(ud) = self.pending_read {
                    if let Some(uring) = &mut self.ring {
                        let _ = uring.complete(ud, pushed_total as i32);
                    }
                }
            }
        }
    }

    pub fn handle_sq(&mut self) {
        loop {
            let sqe = if let Some(ring) = &mut self.ring { ring.next_request() } else { None };
            if let Some(sqe) = sqe {
                match sqe.opcode {
                    glenda::io::uring::IOURING_OP_WRITE => {
                        self.process_tx_ring();
                        let res = if sqe.addr == 0 { 0 } else { sqe.len as i32 };
                        if let Some(ring) = &mut self.ring {
                            let _ = ring.complete(sqe.user_data, res);
                        }
                    }
                    glenda::io::uring::IOURING_OP_READ => {
                        self.pending_read = Some(sqe.user_data);
                        self.process_rx_ring();
                        // Trigger an initial fake CQE to ensure the client checks existing data
                        if let Some(ring) = &mut self.ring {
                            let _ = ring.complete(sqe.user_data, 0);
                        }
                    }
                    _ => {
                        if let Some(ring) = &mut self.ring {
                            let _ = ring.complete(sqe.user_data, -(Error::NotSupported as i32));
                        }
                    }
                }
            } else {
                break;
            }
        }
    }

    pub fn handle_cq(&mut self) {
        // CQ notification from client means client has consumed some entries.
        // We might want to check if TX Ring has space now if we were flow-controlled
    }

    fn process_tx_ring(&mut self) {
        let mut data_to_write = [0u8; 128];
        let mut total_read = 0;

        loop {
            if let Some(tx_ring) = &mut self.tx_ring {
                total_read = tx_ring.pop_slice(&mut data_to_write);
            }

            if total_read > 0 {
                for i in 0..total_read {
                    self.putchar(data_to_write[i]);
                }

                // Notify client that we've consumed TX data
                if let Some(mut uring) = self.ring.take() {
                    let _ = uring.complete(1, 0); // user_data 1 = WRITE
                    self.ring = Some(uring);
                }
            } else {
                break;
            }
        }
    }

    fn getchar(&self) -> Option<u8> {
        unsafe {
            if self.read_reg(LSR) & LSR_DATA_READY != 0 {
                Some(self.read_reg(RBR))
            } else {
                None
            }
        }
    }

    fn putchar(&self, c: u8) {
        unsafe {
            while self.read_reg(LSR) & LSR_THR_EMPTY == 0 {}
            self.write_reg(THR, c);
        }
    }
}

impl UartDriver for Ns16550a {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        for &b in buf {
            self.putchar(b);
        }
        Ok(buf.len())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut count = 0;
        while count < buf.len() {
            if let Some(c) = self.getchar() {
                buf[count] = c;
                count += 1;
            } else {
                break;
            }
        }
        Ok(count)
    }

    fn set_baud_rate(&mut self, baud: u32) {
        Self::set_baud_rate(self, baud);
    }
}
