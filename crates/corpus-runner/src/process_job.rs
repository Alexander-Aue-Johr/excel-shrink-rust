//! Closing the run-owned job also kills helpers blocked inside COM calls.
use anyhow::{Context, Result};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub struct Job(OwnedHandle);

impl Job {
    pub fn new() -> Result<Self> {
        // SAFETY: A null name creates a private job with default security.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        anyhow::ensure!(
            !raw.is_null(),
            "cannot create owned process job: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: CreateJobObjectW returned an owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: This Windows structure permits zero initialization.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: The initialized structure and its byte size agree.
        let ok = unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        anyhow::ensure!(
            ok != 0,
            "cannot configure owned process job: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(handle))
    }

    pub fn add(&self, child: &Child) -> Result<()> {
        // SAFETY: Both retained handles refer to objects owned by this launcher.
        let ok = unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error()).context("assign helper to owner job");
        }
        Ok(())
    }
}
