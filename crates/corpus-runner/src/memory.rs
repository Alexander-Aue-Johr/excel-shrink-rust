//! System-wide physical-memory and commit reserves shared by native runners.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

pub const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub available_physical_bytes: u64,
    pub available_commit_bytes: u64,
    pub total_physical_bytes: u64,
}

impl MemorySnapshot {
    pub fn available_bytes(self) -> u64 {
        self.available_physical_bytes
            .min(self.available_commit_bytes)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MemoryReserve {
    pub bytes: u64,
}

impl MemoryReserve {
    /// Honor an explicit positive reserve. Legacy zero-valued controls retain 512 MiB.
    pub fn from_mib(mib: u64) -> Result<Self> {
        let bytes = (if mib == 0 { 512 } else { mib })
            .checked_mul(MIB)
            .context("Memory reserve overflow")?;
        Ok(Self { bytes })
    }

    /// Keep one eighth of RAM available, bounded between 512 MiB and 8 GiB.
    pub fn for_machine(snapshot: MemorySnapshot) -> Self {
        Self {
            bytes: (snapshot.total_physical_bytes / 8).clamp(512 * MIB, 8192 * MIB),
        }
    }

    pub fn configured(snapshot: MemorySnapshot) -> Result<Self> {
        match std::env::var("EXCEL_SHRINK_SYSTEM_RESERVE_MIB") {
            Ok(value) => Self::from_mib(value.parse().context("Invalid system memory reserve")?),
            Err(std::env::VarError::NotPresent) => Ok(Self::for_machine(snapshot)),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ensure_available(self, snapshot: MemorySnapshot) -> Result<()> {
        ensure!(
            snapshot.available_bytes() >= self.bytes,
            "System memory reserve reached: {} MiB physical available, {} MiB commit headroom, {} MiB required",
            snapshot.available_physical_bytes / MIB,
            snapshot.available_commit_bytes / MIB,
            self.bytes / MIB,
        );
        Ok(())
    }
}

#[cfg(windows)]
pub fn snapshot() -> std::io::Result<MemorySnapshot> {
    use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
    // SAFETY: The structure permits zero initialization and its byte size is supplied.
    let mut information: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    information.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    if unsafe { GetPerformanceInfo(&mut information, information.cb) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let page_bytes = information.PageSize as u64;
    Ok(MemorySnapshot {
        available_physical_bytes: information.PhysicalAvailable as u64 * page_bytes,
        available_commit_bytes: information
            .CommitLimit
            .saturating_sub(information.CommitTotal) as u64
            * page_bytes,
        total_physical_bytes: information.PhysicalTotal as u64 * page_bytes,
    })
}

#[cfg(not(windows))]
pub fn snapshot() -> std::io::Result<MemorySnapshot> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Native Windows memory counters are required for Excel automation",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(physical_mib: u64, commit_mib: u64, total_mib: u64) -> MemorySnapshot {
        MemorySnapshot {
            available_physical_bytes: physical_mib * MIB,
            available_commit_bytes: commit_mib * MIB,
            total_physical_bytes: total_mib * MIB,
        }
    }

    #[test]
    fn abundant_physical_ram_does_not_hide_exhausted_commit() {
        let reserve = MemoryReserve::from_mib(8192).unwrap();
        assert!(reserve
            .ensure_available(memory(58 * 1024, 511, 96 * 1024))
            .is_err());
        assert!(reserve
            .ensure_available(memory(511, 58 * 1024, 96 * 1024))
            .is_err());
        assert!(reserve
            .ensure_available(memory(8192, 8192, 96 * 1024))
            .is_ok());
    }

    #[test]
    fn automatic_reserves_fit_small_and_large_machines() {
        assert_eq!(
            MemoryReserve::for_machine(memory(0, 0, 8192)).bytes,
            1024 * MIB
        );
        assert_eq!(
            MemoryReserve::for_machine(memory(0, 0, 96 * 1024)).bytes,
            8192 * MIB
        );
        assert_eq!(MemoryReserve::from_mib(0).unwrap().bytes, 512 * MIB);
        assert!(MemoryReserve::from_mib(u64::MAX).is_err());
    }

    #[test]
    fn explicit_small_reserves_check_both_physical_and_commit_headroom() {
        let reserve = MemoryReserve::from_mib(265).unwrap();
        assert_eq!(reserve.bytes, 265 * MIB);
        assert!(reserve.ensure_available(memory(265, 265, 8192)).is_ok());
        assert!(reserve.ensure_available(memory(264, 1024, 8192)).is_err());
        assert!(reserve.ensure_available(memory(1024, 264, 8192)).is_err());
    }
}
