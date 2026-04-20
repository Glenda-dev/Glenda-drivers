//! NS16550A Configuration

/// UART clock frequency (in Hz)
/// For QEMU virt machine, the UART clock is typically dependent on the platform.
/// Standard PC UART is usually 1.8432 MHz.
pub const UART_CLOCK: u32 = 1_843_200;

/// Default baud rate
pub const DEFAULT_BAUD_RATE: u32 = 38400;

/// RX software buffer soft limit (bytes)
pub const RX_BUFFER_SOFT_LIMIT: usize = 4096;

/// TX ring read chunk size per iteration (bytes)
pub const TX_RING_CHUNK: usize = 128;

/// RX bytes budget processed per IRQ event (bytes)
pub const RX_IRQ_BUDGET_BYTES: usize = 512;

/// RX bytes budget interleaved during TX processing (bytes)
pub const RX_TX_INTERLEAVE_BUDGET_BYTES: usize = 64;

/// TX bytes budget when handling THR-empty IRQ (bytes)
pub const TX_IRQ_BUDGET_BYTES: usize = 512;

/// TX bytes budget when handling SQ WRITE requests (bytes)
pub const TX_SQ_BUDGET_BYTES: usize = 1024;

/// Max IRQ events handled in one IRQ dispatch round
pub const IRQ_EVENT_BUDGET: usize = 64;

/// Sparse log throttle interval for counters
pub const LOG_THROTTLE_EVERY: usize = 1024;

/// Calculate divisor for a given baud rate
pub const fn calculate_divisor(baud: u32) -> u16 {
    (UART_CLOCK / (16 * baud)) as u16
}
