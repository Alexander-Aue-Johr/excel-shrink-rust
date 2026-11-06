use serde_json::{json, Value};
use std::{fs, path::Path};

pub(super) use crate::native_process::process_alive;
#[cfg(windows)]
use crate::native_process::running_handle_matches;

/// Last resort if the runner cannot finish its bounded worker and Excel cleanup.
/// The creation time is checked on the same handle used for termination.
#[cfg(windows)]
pub(super) fn terminate_verified_process(
    pid: i64,
    expected_start_ms: i64,
) -> std::io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE,
        },
    };
    let pid = u32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid runner PID")
        })?;
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let result = match running_handle_matches(handle, expected_start_ms) {
        Some(false) => Ok(false),
        Some(true) => {
            if unsafe { TerminateProcess(handle, 130) } == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(true)
            }
        }
        None => Err(std::io::Error::last_os_error()),
    };
    unsafe { CloseHandle(handle) };
    result
}

#[cfg(not(windows))]
pub(super) fn terminate_verified_process(
    _pid: i64,
    _expected_start_ms: i64,
) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "verified process termination requires Windows",
    ))
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
                "commit_limit_bytes": status.ullTotalPageFile,
                "commit_available_bytes": status.ullAvailPageFile,
                "commit_used_bytes": status.ullTotalPageFile.saturating_sub(status.ullAvailPageFile),
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Child, Command, Stdio};
    use windows_sys::Win32::{
        Foundation::{FILETIME, HANDLE},
        System::Threading::GetProcessTimes,
    };

    struct TestChild(Child);

    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn termination_checks_creation_time_on_the_killed_process() -> std::io::Result<()> {
        let mut child = TestChild(
            Command::new("powershell.exe")
                .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let mut stamps = [FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        }; 4];
        let [created, exited, kernel, user] = &mut stamps;
        if unsafe {
            GetProcessTimes(
                child.0.as_raw_handle() as HANDLE,
                created,
                exited,
                kernel,
                user,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let started = ((ticks - 116_444_736_000_000_000) / 10_000) as i64;
        let pid = i64::from(child.0.id());

        assert_eq!(process_alive(pid, started), Some(true));
        assert!(!terminate_verified_process(pid, started - 1000)?);
        assert_eq!(process_alive(pid, started), Some(true));
        assert!(terminate_verified_process(pid, started)?);
        child.0.wait()?;
        assert_eq!(process_alive(pid, started), Some(false));
        Ok(())
    }
}
