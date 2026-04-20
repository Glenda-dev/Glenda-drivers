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

#[derive(Default)]
pub struct UartStats {
    pub rx_hw_lsr_overrun: usize,
    pub rx_sw_drop: usize,
    pub rx_ring_full_break: usize,
    pub rx_budget_hit: usize,
    pub tx_budget_hit: usize,
    pub irq_event_budget_hit: usize,
    pub irq_rx_bytes: usize,
    pub irq_tx_bytes: usize,
    pub irq_rx_batches: usize,
}

#[inline]
fn should_log_sparse(count: usize) -> bool {
    count == 1 || count.is_power_of_two() || count % LOG_THROTTLE_EVERY == 0
}

pub struct Ns16550a {
    pub base: usize,
    pub irq: IrqHandler,
    pub ring: Option<IoUringServer>,
    pub shm: Option<SharedMemory>,
    pub rx_ring: Option<&'static mut ShmRingBuffer>,
    pub tx_ring: Option<&'static mut ShmRingBuffer>,
    pub rx_buffer: VecDeque<u8>,
    pub pending_read: Option<usize>,
    pub stats: UartStats,
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
            rx_buffer: VecDeque::with_capacity(RX_BUFFER_SOFT_LIMIT),
            pending_read: None,
            stats: UartStats::default(),
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
            // Enable RX + Receiver Line Status interrupt
            self.write_reg(IER, IER_RX_ENABLE | IER_RLS_ENABLE);
        }
        log!(
            "config: baud={} ier={:#x} fifo={:#x} rx_buf_limit={} rx_irq_budget={} tx_irq_budget={}",
            DEFAULT_BAUD_RATE,
            IER_RX_ENABLE | IER_RLS_ENABLE,
            FCR_FIFO_ENABLE | FCR_FIFO_RX_RESET | FCR_FIFO_TX_RESET | FCR_FIFO_TRIGGER_14,
            RX_BUFFER_SOFT_LIMIT,
            RX_IRQ_BUDGET_BYTES,
            TX_IRQ_BUDGET_BYTES
        );
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

            // Enable FIFO, clear them, and set RX trigger to 14 bytes
            self.write_reg(
                FCR,
                FCR_FIFO_ENABLE | FCR_FIFO_RX_RESET | FCR_FIFO_TX_RESET | FCR_FIFO_TRIGGER_14,
            );

            // IRQs enabled, RTS/DTR set
            self.write_reg(MCR, MCR_OUT2 | MCR_RTS | MCR_DTR);
        }
    }

    pub fn handle_irq(&mut self) -> Result<(), Error> {
        let mut has_data = false;
        let mut irq_events = 0usize;
        loop {
            if irq_events >= IRQ_EVENT_BUDGET {
                self.stats.irq_event_budget_hit += 1;
                if should_log_sparse(self.stats.irq_event_budget_hit) {
                    warn!(
                        "IRQ event budget hit (count={}), postpone remaining events",
                        self.stats.irq_event_budget_hit
                    );
                }
                break;
            }

            unsafe {
                let iir = self.read_reg(IIR);
                if iir & IIR_NO_INTERRUPT != 0 {
                    break;
                }

                irq_events += 1;
                let id = iir & IIR_ID_MASK;
                if id == IIR_RX_DATA_READY || id == IIR_TIMEOUT || id == IIR_RLS {
                    self.stats.irq_rx_batches += 1;
                    has_data |= self.drain_rx_fifo_to_sw_buffer(RX_IRQ_BUDGET_BYTES) > 0;
                } else if id == IIR_THR_EMPTY {
                    let _ = self.process_tx_ring(TX_IRQ_BUDGET_BYTES);
                } else {
                    self.record_lsr_errors();
                }
            }
        }

        if has_data || !self.rx_buffer.is_empty() {
            self.process_rx_ring();
        }
        Ok(())
    }

    fn record_lsr_errors(&mut self) {
        unsafe {
            let lsr = self.read_reg(LSR);
            if lsr & LSR_OVERRUN_ERROR != 0 {
                self.stats.rx_hw_lsr_overrun += 1;
                if should_log_sparse(self.stats.rx_hw_lsr_overrun) {
                    warn!(
                        "LSR overrun detected count={} (rx_sw_drop={}, rx_ring_full={})",
                        self.stats.rx_hw_lsr_overrun,
                        self.stats.rx_sw_drop,
                        self.stats.rx_ring_full_break
                    );
                }
            }
        }
    }

    fn drain_rx_fifo_to_sw_buffer(&mut self, budget: usize) -> usize {
        let mut drained = 0usize;
        if budget == 0 {
            return drained;
        }

        self.record_lsr_errors();

        while drained < budget {
            let Some(c) = self.getchar() else {
                break;
            };

            if self.rx_buffer.len() < RX_BUFFER_SOFT_LIMIT {
                self.rx_buffer.push_back(c);
                self.stats.irq_rx_bytes += 1;
                drained += 1;
            } else {
                self.stats.rx_sw_drop += 1;
                if should_log_sparse(self.stats.rx_sw_drop) {
                    warn!(
                        "RX software buffer full, dropping byte {:#x} (count={})",
                        c, self.stats.rx_sw_drop
                    );
                }
            }
        }

        if drained >= budget {
            unsafe {
                if self.read_reg(LSR) & LSR_DATA_READY != 0 {
                    self.stats.rx_budget_hit += 1;
                    if should_log_sparse(self.stats.rx_budget_hit) {
                        warn!(
                            "RX budget hit count={} (budget={}, buffered={})",
                            self.stats.rx_budget_hit,
                            budget,
                            self.rx_buffer.len()
                        );
                    }
                }
            }
        }

        drained
    }

    fn process_rx_ring(&mut self) {
        // Try to push to SHM Ring Buffer first
        if let Some(ring) = &mut self.rx_ring {
            let mut pushed_total = 0;
            while !self.rx_buffer.is_empty() {
                let (front_len, pushed) = {
                    let (front, _) = self.rx_buffer.as_slices();
                    if front.is_empty() {
                        (0usize, 0usize)
                    } else {
                        (front.len(), ring.push_slice(front))
                    }
                };

                if front_len == 0 {
                    break;
                }

                if pushed == 0 {
                    self.stats.rx_ring_full_break += 1;
                    if should_log_sparse(self.stats.rx_ring_full_break) {
                        warn!(
                            "RX shm ring full, postpone {} buffered bytes (count={})",
                            self.rx_buffer.len(),
                            self.stats.rx_ring_full_break
                        );
                    }
                    break; // Ring full
                }

                for _ in 0..pushed {
                    let _ = self.rx_buffer.pop_front();
                }
                pushed_total += pushed;

                // Partial push implies ring is full in this round.
                if pushed < front_len {
                    self.stats.rx_ring_full_break += 1;
                    if should_log_sparse(self.stats.rx_ring_full_break) {
                        warn!(
                            "RX shm ring reached capacity mid-batch, {} bytes still buffered (count={})",
                            self.rx_buffer.len(),
                            self.stats.rx_ring_full_break
                        );
                    }
                    break;
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
                        let written = self.process_tx_ring(TX_SQ_BUDGET_BYTES);
                        let res = if sqe.addr == 0 { written as i32 } else { sqe.len as i32 };
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

        if !self.rx_buffer.is_empty() {
            self.process_rx_ring();
        }
    }

    pub fn handle_cq(&mut self) {
        // CQ notification from client means client has consumed some entries.
        // Opportunistically flush pending RX data into shm ring.
        if !self.rx_buffer.is_empty() {
            self.process_rx_ring();
        }
    }

    fn process_tx_ring(&mut self, budget: usize) -> usize {
        let mut data_to_write = [0u8; TX_RING_CHUNK];
        let mut total_written = 0;
        let mut remaining_budget = budget;

        if remaining_budget == 0 {
            return 0;
        }

        while remaining_budget > 0 {
            let chunk_len = core::cmp::min(data_to_write.len(), remaining_budget);
            let total_read = if let Some(tx_ring) = &mut self.tx_ring {
                tx_ring.pop_slice(&mut data_to_write[..chunk_len])
            } else {
                0
            };

            if total_read > 0 {
                for i in 0..total_read {
                    self.putchar(data_to_write[i]);
                }

                total_written += total_read;
                self.stats.irq_tx_bytes += total_read;
                remaining_budget = remaining_budget.saturating_sub(total_read);

                // Interleave a quick RX drain chance between TX chunks.
                if self.drain_rx_fifo_to_sw_buffer(RX_TX_INTERLEAVE_BUDGET_BYTES) > 0 {
                    self.process_rx_ring();
                }
            } else {
                break;
            }
        }

        if remaining_budget == 0 {
            let has_pending_tx = self.tx_ring.as_ref().map(|r| r.len() > 0).unwrap_or(false);
            if has_pending_tx {
                self.stats.tx_budget_hit += 1;
                if should_log_sparse(self.stats.tx_budget_hit) {
                    warn!(
                        "TX budget hit count={} (budget={}, pending_tx={})",
                        self.stats.tx_budget_hit,
                        budget,
                        self.tx_ring.as_ref().map(|r| r.len()).unwrap_or(0)
                    );
                }
            }
        }

        total_written
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
