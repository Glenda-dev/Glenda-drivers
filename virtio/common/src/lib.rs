#![no_std]

pub mod consts;
pub mod queue;
pub mod transport;

pub use consts::*;
pub use queue::*;
pub use transport::*;

#[derive(Debug)]
pub enum VirtIOError {
    DeviceNotFound,
    InvalidHeader,
    QueueTooSmall,
    OOM,
}

pub type Result<T> = core::result::Result<T, VirtIOError>;
