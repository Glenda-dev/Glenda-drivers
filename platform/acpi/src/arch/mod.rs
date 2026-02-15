#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub mod x86;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub use x86::parse_madt;

#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
pub mod riscv;
#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
pub use riscv::parse_madt;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;
#[cfg(any(target_arch = "aarch64"))]
pub use aarch64::parse_madt;
