//! NS16550A Configuration

/// UART clock frequency (in Hz)
/// For QEMU virt machine, the UART clock is typically dependent on the platform.
/// Standard PC UART is usually 1.8432 MHz.
pub const UART_CLOCK: u32 = 1_843_200;

/// Default baud rate
pub const DEFAULT_BAUD_RATE: u32 = 38400;

/// Calculate divisor for a given baud rate
pub const fn calculate_divisor(baud: u32) -> u16 {
    (UART_CLOCK / (16 * baud)) as u16
}
