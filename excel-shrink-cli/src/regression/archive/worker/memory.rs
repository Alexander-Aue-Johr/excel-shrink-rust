//! Native process counters: private commit on Windows, resident memory on Linux.
use std::io;

pub(super) fn configured_reserve_bytes() -> io::Result<u64> {
    let value = std::env::var("EXCEL_SHRINK_WORKER_MEMORY_MIB");
    match value {
        Ok(value) => parse_reserve(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_reserve(None),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

fn parse_reserve(value: Option<&str>) -> io::Result<u64> {
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
    fn memory_reserve_setting_rejects_invalid_values() {
        assert_eq!(parse_reserve(None).unwrap(), 4096 * 1_048_576);
        assert_eq!(parse_reserve(Some("8192")).unwrap(), 8192 * 1_048_576);
        assert_eq!(parse_reserve(Some("64")).unwrap(), 64 * 1_048_576);
        for value in ["", "0", "63", "-1", "4096.5", "unlimited", "1048577"] {
            assert!(parse_reserve(Some(value)).is_err(), "accepted {value:?}");
        }
    }


}
