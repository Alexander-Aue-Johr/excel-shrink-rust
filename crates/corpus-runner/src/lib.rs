//! Shared source identities, durable artifacts, process ownership and resource budgets.
pub mod artifacts;
pub mod memory;
pub mod native_process;
pub mod package_sizes;
#[cfg(windows)]
pub mod process_job;
pub mod sources;
pub mod telemetry;
