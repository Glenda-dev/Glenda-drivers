//! NS16550A Register Offsets and Bit Flags

/// Transmitter Holding Register (Write)
pub const THR: usize = 0;
/// Receiver Buffer Register (Read)
pub const RBR: usize = 0;
/// Divisor Latch Low (DLAB=1)
pub const DLL: usize = 0;
/// Divisor Latch High (DLAB=1)
pub const DLM: usize = 1;
/// Interrupt Enable Register
pub const IER: usize = 1;
/// Interrupt Identification Register (Read)
pub const IIR: usize = 2;
/// FIFO Control Register (Write)
pub const FCR: usize = 2;
/// Line Control Register
pub const LCR: usize = 3;
/// Modem Control Register
pub const MCR: usize = 4;
/// Line Status Register
pub const LSR: usize = 5;
/// Modem Status Register
pub const MSR: usize = 6;
/// Scratch Register
pub const SCR: usize = 7;

// IER bits
pub const IER_RX_ENABLE: u8 = 1 << 0;
pub const IER_TX_ENABLE: u8 = 1 << 1;
pub const IER_RLS_ENABLE: u8 = 1 << 2; // Receiver Line Status
pub const IER_MS_ENABLE: u8 = 1 << 3; // Modem Status

// FCR bits
pub const FCR_FIFO_ENABLE: u8 = 1 << 0;
pub const FCR_FIFO_RX_RESET: u8 = 1 << 1;
pub const FCR_FIFO_TX_RESET: u8 = 1 << 2;
pub const FCR_FIFO_DMA_MODE: u8 = 1 << 3;
pub const FCR_FIFO_64BYTE: u8 = 1 << 5;

// LCR bits
pub const LCR_DATA_BITS_5: u8 = 0;
pub const LCR_DATA_BITS_6: u8 = 1;
pub const LCR_DATA_BITS_7: u8 = 2;
pub const LCR_DATA_BITS_8: u8 = 3;
pub const LCR_STOP_BITS_1: u8 = 0;
pub const LCR_STOP_BITS_2: u8 = 1 << 2;
pub const LCR_PARITY_NONE: u8 = 0;
pub const LCR_PARITY_ODD: u8 = 1 << 3;
pub const LCR_PARITY_EVEN: u8 = 3 << 3;
pub const LCR_DLAB: u8 = 1 << 7;

// MCR bits
pub const MCR_DTR: u8 = 1 << 0;
pub const MCR_RTS: u8 = 1 << 1;
pub const MCR_OUT1: u8 = 1 << 2;
pub const MCR_OUT2: u8 = 1 << 3;
pub const MCR_LOOPBACK: u8 = 1 << 4;

// LSR bits
pub const LSR_DATA_READY: u8 = 1 << 0;
pub const LSR_OVERRUN_ERROR: u8 = 1 << 1;
pub const LSR_PARITY_ERROR: u8 = 1 << 2;
pub const LSR_FRAMING_ERROR: u8 = 1 << 3;
pub const LSR_BREAK_INTERRUPT: u8 = 1 << 4;
pub const LSR_THR_EMPTY: u8 = 1 << 5;
pub const LSR_TRANSMITTER_EMPTY: u8 = 1 << 6;
pub const LSR_FIFO_ERROR: u8 = 1 << 7;

// IIR bits
pub const IIR_NO_INTERRUPT: u8 = 1 << 0;
pub const IIR_ID_MASK: u8 = 0x0E;
