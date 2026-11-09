//! Loopback web interface for the shared native preparation backends.
#[cfg(windows)]
mod server;
#[cfg(windows)]
pub use server::{serve, Defaults};
