//! Stop admitting Excel work before diagnostics exhaust the Windows temp volume.
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
const MINIMUM_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) struct Guard {
    last_sample: Option<Instant>,
    state: Value,
}

impl Guard {
    pub(super) fn new() -> Self {
        Self {
            last_sample: None,
            state: Value::Null,
        }
    }

    pub(super) fn allows_admission(&mut self, queue: &mut Value, now_ms: u64) -> bool {
        if self
            .last_sample
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            self.state = sample();
            self.state["sampled_unix_ms"] = json!(now_ms);
            self.last_sample = Some(Instant::now());
        }
        queue["admission_guard"] = self.state.clone();
        self.state["blocked"] != true
    }
}

#[cfg(any(windows, test))]
fn decision(free: std::io::Result<u64>) -> Value {
    match free {
        Ok(bytes) => json!({
            "blocked":bytes < MINIMUM_FREE_BYTES,
            "reason":if bytes < MINIMUM_FREE_BYTES { "low_disk_space" } else { "sufficient_disk_space" },
            "free_bytes":bytes,"minimum_free_bytes":MINIMUM_FREE_BYTES,
            "message":if bytes < MINIMUM_FREE_BYTES {
                "New groups wait: the Excel diagnostic volume has less than 2 GiB free. Existing work may finish; admission resumes when space is available."
            } else { "Excel diagnostic volume has enough free space." }
        }),
        Err(error) => json!({
            "blocked":true,"reason":"disk_space_unavailable","free_bytes":null,
            "minimum_free_bytes":MINIMUM_FREE_BYTES,"error":error.to_string(),
            "message":"New groups wait because free space on the Excel diagnostic volume could not be verified. The query will be retried automatically."
        }),
    }
}

#[cfg(windows)]
fn sample() -> Value {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return decision(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "LOCALAPPDATA is not set",
        )));
    };
    let path = PathBuf::from(local).join("Temp");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // SAFETY: the UTF-16 path is terminated and the output pointer remains valid.
    // Optional total/free outputs may be null according to the Win32 API.
    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    let mut state = decision(if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(available)
    });
    state["path"] = json!(path);
    state
}

#[cfg(not(windows))]
fn sample() -> Value {
    // WSL's filesystem counters do not establish Windows Excel's diagnostic space.
    json!({"blocked":false,"reason":"not_native_windows","free_bytes":null})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_space_blocks_new_admissions_and_recovers_at_threshold() {
        assert_eq!(decision(Ok(MINIMUM_FREE_BYTES - 1))["blocked"], true);
        assert_eq!(decision(Ok(0))["blocked"], true);
        assert_eq!(decision(Ok(MINIMUM_FREE_BYTES))["blocked"], false);
        assert_eq!(decision(Ok(10 * MINIMUM_FREE_BYTES))["blocked"], false);
    }

    #[test]
    fn query_failures_are_explicit_and_never_fabricate_free_space() {
        let state = decision(Err(std::io::Error::from_raw_os_error(5)));
        assert_eq!(state["blocked"], true);
        assert_eq!(state["reason"], "disk_space_unavailable");
        assert!(state["free_bytes"].is_null());
        assert!(state["error"].is_string());
    }
}
