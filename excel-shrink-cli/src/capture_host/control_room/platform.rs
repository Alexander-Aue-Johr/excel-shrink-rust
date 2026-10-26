use serde_json::{json, Value};
use std::{fs, path::Path};

/// Unknown means access could not be verified; quiet heartbeats are not evidence
/// that a process exited. PID and creation time must both match.
#[cfg(windows)]
pub(super) fn process_alive(pid: i64, expected_start_ms: i64) -> Option<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME, WAIT_TIMEOUT},
        System::Threading::{
            GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        },
    };
    let pid = u32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return (unsafe { GetLastError() } == ERROR_INVALID_PARAMETER).then_some(false);
    }
    let result = (|| {
        match unsafe { WaitForSingleObject(handle, 0) } {
            0 => return Some(false),
            WAIT_TIMEOUT => {}
            _ => return None,
        }
        let mut stamps = [FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        }; 4];
        let [created, exited, kernel, user] = &mut stamps;
        if unsafe { GetProcessTimes(handle, created, exited, kernel, user) } == 0 {
            return None;
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let actual_ms = ticks.checked_sub(116_444_736_000_000_000)? / 10_000;
        Some(i128::from(actual_ms).abs_diff(i128::from(expected_start_ms)) <= 1)
    })();
    unsafe { CloseHandle(handle) };
    result
}

#[cfg(not(windows))]
pub(super) fn process_alive(_pid: i64, _expected_start_ms: i64) -> Option<bool> {
    None
}

pub(super) fn memory_snapshot() -> Value {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            return json!({
                "total_bytes": status.ullTotalPhys,
                "available_bytes": status.ullAvailPhys,
                "load_percent": status.dwMemoryLoad,
                "source": "GlobalMemoryStatusEx"
            });
        }
    }
    json!({"total_bytes": null, "available_bytes": null, "load_percent": null, "source": "unavailable"})
}

pub(super) fn output_idle(root: &Path) -> bool {
    let Some(parent) = root.parent() else {
        return false;
    };
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(parent.join(".run.lock"))
    {
        Ok(file) => file.try_lock().is_ok(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

pub(super) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
