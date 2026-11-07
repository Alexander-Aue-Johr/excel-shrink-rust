//! Shared PID-and-creation-time checks for telemetry and verified cleanup.

/// Unknown means access could not be verified; quiet heartbeats are not evidence
/// that a process exited. PID and creation time must both match.
#[cfg(windows)]
pub(crate) fn process_alive(pid: i64, expected_start_ms: i64) -> Option<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE},
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
    let result = running_handle_matches(handle, expected_start_ms);
    unsafe { CloseHandle(handle) };
    result
}

#[cfg(windows)]
pub(crate) fn running_handle_matches(
    handle: windows_sys::Win32::Foundation::HANDLE,
    expected_start_ms: i64,
) -> Option<bool> {
    use windows_sys::Win32::{Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject};
    match unsafe { WaitForSingleObject(handle, 0) } {
        0 => return Some(false),
        WAIT_TIMEOUT => {}
        _ => return None,
    }
    let actual_ms = creation_time_ms(handle)?;
    Some(i128::from(actual_ms).abs_diff(i128::from(expected_start_ms)) <= 1)
}

#[cfg(not(windows))]
pub(crate) fn process_alive(_pid: i64, _expected_start_ms: i64) -> Option<bool> {
    None
}

/// Creation time of this process, using the same identity precision as cleanup.
#[cfg(windows)]
pub(crate) fn current_started_unix_ms() -> Option<i64> {
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle.
    creation_time_ms(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
}

#[cfg(windows)]
fn creation_time_ms(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<i64> {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
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
    i64::try_from(actual_ms).ok()
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn current_creation_time_matches_only_the_current_identity() {
        let started = super::current_started_unix_ms().unwrap();
        assert_eq!(
            super::process_alive(i64::from(std::process::id()), started),
            Some(true)
        );
        assert_eq!(
            super::process_alive(i64::from(std::process::id()), started - 1000),
            Some(false)
        );
    }
}
