//! Run-owned native Excel automation shared by CLI and web applications.
pub use corpus_runner::native_process;
#[cfg(windows)]
pub use corpus_runner::process_job;
pub mod metrics;
pub mod shell;
pub use metrics::ExcelOpenMetrics;
#[cfg(windows)]
pub mod pool;
