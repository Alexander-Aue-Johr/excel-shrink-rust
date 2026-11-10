//! Loopback web interface for the shared native preparation backends.
#[cfg(windows)]
mod server;
#[cfg(windows)]
pub use server::{serve, serve_sheetshrink, serve_with_browser, Defaults};
