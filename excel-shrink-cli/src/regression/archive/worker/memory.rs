//! Native process counters: private commit on Windows, resident memory on Linux.
use std::io;

pub(super) fn limit_bytes() -> io::Result<u64> {
    let value = std::env::var("EXCEL_SHRINK_WORKER_MEMORY_MIB");
    match value {
        Ok(value) => parse_limit(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_limit(None),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

fn parse_limit(value: Option<&str>) -> io::Result<u64> {
    value
        .unwrap_or("4096")
        .parse::<u64>()
        .ok()
        .filter(|mib| (64..=1_048_576).contains(mib))
        .map(|mib| mib * 1_048_576)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "EXCEL_SHRINK_WORKER_MEMORY_MIB must be an integer between 64 and 1048576 MiB",
            )
        })
}

#[cfg(windows)]
pub(super) fn enforce_limit(bytes: u64) -> io::Result<std::os::windows::io::OwnedHandle> {
    use std::ffi::c_void;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    // Windows ABI layouts from JOBOBJECT_EXTENDED_LIMIT_INFORMATION. The I/O
    // counters are six ULONGLONGs; the memory limits and affinity are SIZE_T.
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            information: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x100;
    const JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    let limits = ExtendedLimits {
        basic: BasicLimits {
            // Only this Rust worker is constrained. Excel launchers retain their
            // separate job ownership, cleanup, and configurable Excel limits.
            limit_flags: JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
            ..Default::default()
        },
        process_memory_limit: usize::try_from(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        ..Default::default()
    };
    // SAFETY: null arguments create an unnamed job with a non-inheritable handle.
    let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw_job.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateJobObjectW returned a new handle; OwnedHandle closes it once.
    let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };
    // SAFETY: the correctly sized buffer and both handles are valid for the calls.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            (&limits as *const ExtendedLimits).cast(),
            std::mem::size_of::<ExtendedLimits>() as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    eprintln!(
        "[archive] Windows worker allocation limit: {} MiB",
        bytes / 1_048_576
    );
    Ok(job)
}

#[derive(Clone, Copy)]
pub(super) struct Usage {
    pub current_bytes: u64,
    pub peak_bytes: u64,
}

pub(super) const METRIC: &str = if cfg!(windows) {
    "private committed"
} else {
    "resident"
};

#[cfg(windows)]
pub(super) fn usage() -> io::Result<Usage> {
    // PROCESS_MEMORY_COUNTERS_EX; SIZE_T is pointer-sized on both Win32 and Win64.
    // https://learn.microsoft.com/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    let size = std::mem::size_of::<ProcessMemoryCounters>() as u32;
    let mut counters = ProcessMemoryCounters {
        cb: size,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle refers to this process and needs no closing. The
    // writable buffer has the documented layout and remains valid for the call.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Usage {
        // Private commit includes paged-out allocations, which also consume the
        // system commit limit. Working set alone would miss that memory pressure.
        current_bytes: counters.private_usage as u64,
        peak_bytes: counters.peak_pagefile_usage as u64,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn usage() -> io::Result<Usage> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let read_bytes = |key| {
        status
            .lines()
            .find_map(|line| {
                line.strip_prefix(key)?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "missing process memory counter")
            })
    };
    Ok(Usage {
        current_bytes: read_bytes("VmRSS:")?,
        peak_bytes: read_bytes("VmHWM:")?,
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn usage() -> io::Result<Usage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "worker memory counters are unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_setting_rejects_invalid_values_instead_of_disabling_the_limit() {
        assert_eq!(parse_limit(None).unwrap(), 4096 * 1_048_576);
        assert_eq!(parse_limit(Some("8192")).unwrap(), 8192 * 1_048_576);
        assert_eq!(parse_limit(Some("64")).unwrap(), 64 * 1_048_576);
        for value in ["", "0", "63", "-1", "4096.5", "unlimited", "1048577"] {
            assert!(parse_limit(Some(value)).is_err(), "accepted {value:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_allocations_above_the_worker_limit() -> io::Result<()> {
        // The job belongs only to this child; never cap the cargo/test runner.
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "regression::archive::worker::memory::tests::hard_limit_test_child",
                "--nocapture",
            ])
            .env("EXCEL_SHRINK_HARD_LIMIT_TEST_CHILD", "worker")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn hard_limit_test_child() -> io::Result<()> {
        let Ok(role) = std::env::var("EXCEL_SHRINK_HARD_LIMIT_TEST_CHILD") else {
            return Ok(());
        };
        // Reserve+commit without touching pages: this consumes only a small
        // amount of physical RAM and cannot trigger Rust's allocation abort.
        fn can_commit(bytes: usize) -> bool {
            use std::ffi::c_void;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn VirtualAlloc(
                    address: *mut c_void,
                    size: usize,
                    kind: u32,
                    protect: u32,
                ) -> *mut c_void;
                fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
            }
            // SAFETY: null requests a fresh allocation; release uses that exact
            // returned address with MEM_RELEASE and the required zero length.
            let pointer = unsafe { VirtualAlloc(std::ptr::null_mut(), bytes, 0x3000, 0x04) };
            if pointer.is_null() {
                return false;
            }
            assert_ne!(unsafe { VirtualFree(pointer, 0, 0x8000) }, 0);
            true
        }
        if role == "descendant" {
            assert!(
                can_commit(128 * 1_048_576),
                "child inherited Rust worker memory cap"
            );
            return Ok(());
        }
        let _limit = enforce_limit(64 * 1_048_576)?;
        assert!(
            !can_commit(128 * 1_048_576),
            "job did not enforce its memory cap"
        );
        assert!(can_commit(8 * 1_048_576), "job rejected a small allocation");

        // Excel and its launchers must remain under their existing independent
        // limits; process descendants should not inherit this Rust worker job.
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "regression::archive::worker::memory::tests::hard_limit_test_child",
                "--nocapture",
            ])
            .env("EXCEL_SHRINK_HARD_LIMIT_TEST_CHILD", "descendant")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        Ok(())
    }
}
