//! Reserve preparation space and protect the Windows system volume from exhaustion.

use anyhow::Result;
use serde::Serialize;
use std::path::Path;

const GIB: u64 = 1024 * 1024 * 1024;
const MINIMUM_FREE_BYTES: u64 = 8 * GIB;
const FIXED_HEADROOM_BYTES: u64 = 2 * GIB;
// Preparation may hold the extracted source, three outputs, a repaired
// baseline, and a bundle at once. This is admission headroom, not a hard cap:
// some workbooks expand far beyond their archive size while being processed.
const PEAK_SOURCE_MULTIPLIER: u64 = 8;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct GuardState {
    pub volume: &'static str,
    pub free_bytes: u64,
    pub required_free_bytes: u64,
    pub input_bytes: Option<u64>,
    pub blocked: bool,
}

pub(super) fn check(output: &Path, input_bytes: Option<u64>) -> Result<GuardState> {
    check_with_active(output, input_bytes, 0)
}

/// Reserve the shared disk floor and the combined peak for already admitted
/// preparations plus the next workbook. The active bytes are source XLSX sizes.
pub(super) fn check_with_active(
    output: &Path,
    input_bytes: Option<u64>,
    active_source_bytes: u64,
) -> Result<GuardState> {
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot").ok_or_else(|| {
            anyhow::anyhow!("SystemRoot is unavailable for the system disk check")
        })?;
        let system_free = available_bytes(Path::new(&system_root))?;
        if let Some(blocked) = system_volume_guard(system_free, input_bytes) {
            return Ok(blocked);
        }
    }
    let free_bytes = available_bytes(output)?;
    let required_free_bytes = required_free_bytes(Some(
        active_source_bytes.saturating_add(input_bytes.unwrap_or(0)),
    ));
    Ok(GuardState {
        volume: "study output volume",
        free_bytes,
        required_free_bytes,
        input_bytes,
        blocked: free_bytes < required_free_bytes,
    })
}

/// System caches need a fixed reserve even when workbook artifacts live elsewhere.
#[cfg(any(windows, test))]
fn system_volume_guard(free_bytes: u64, input_bytes: Option<u64>) -> Option<GuardState> {
    (free_bytes < MINIMUM_FREE_BYTES).then_some(GuardState {
        volume: "Windows system volume",
        free_bytes,
        required_free_bytes: MINIMUM_FREE_BYTES,
        input_bytes,
        blocked: true,
    })
}

fn required_free_bytes(input_bytes: Option<u64>) -> u64 {
    let estimated_peak = input_bytes
        .unwrap_or(0)
        .saturating_mul(PEAK_SOURCE_MULTIPLIER)
        .saturating_add(FIXED_HEADROOM_BYTES);
    MINIMUM_FREE_BYTES.max(estimated_peak)
}

#[cfg(unix)]
fn available_bytes(path: &Path) -> Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` is writable for one statvfs result.
    let status = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if status != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: statvfs returned success and initialized the output structure.
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(windows)]
fn available_bytes(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: `path` is NUL-terminated and the optional output pointer is writable.
    let status = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_the_system_disk_independently_of_workbook_size() {
        let blocked = system_volume_guard(8 * GIB - 1, Some(123)).unwrap();
        assert_eq!(blocked.volume, "Windows system volume");
        assert!(blocked.blocked);
        assert_eq!(blocked.input_bytes, Some(123));
        assert_eq!(blocked.required_free_bytes, 8 * GIB);
        assert!(system_volume_guard(8 * GIB, Some(u64::MAX)).is_none());
    }

    #[test]
    fn reserves_space_for_the_next_bundle_and_temporary_outputs() {
        assert_eq!(required_free_bytes(None), 8 * GIB);
        assert_eq!(required_free_bytes(Some(GIB)), 10 * GIB);
        assert_eq!(required_free_bytes(Some(u64::MAX)), u64::MAX);
    }

    #[test]
    fn concurrent_preparations_share_the_fixed_disk_reserve() -> Result<()> {
        let output = std::env::temp_dir();
        let state = check_with_active(&output, Some(GIB), GIB)?;
        assert_eq!(state.required_free_bytes, 18 * GIB);
        assert_eq!(
            check_with_active(&output, Some(0), 0)?.required_free_bytes,
            8 * GIB
        );
        Ok(())
    }

    #[test]
    fn checks_the_actual_output_volume() -> Result<()> {
        let state = check(&std::env::temp_dir(), None)?;
        assert!(state.free_bytes > 0);
        assert_eq!(state.blocked, state.free_bytes < state.required_free_bytes);
        Ok(())
    }
}
