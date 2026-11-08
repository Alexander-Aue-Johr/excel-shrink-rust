//! Exclusive browser profiles survive successful sessions; interrupted leases stay quarantined.
use anyhow::{ensure, Context, Result};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

const MAX_PROFILE_SLOTS: usize = 128;

pub(super) struct ProfileLease {
    pub directory: PathBuf,
    _lock: File,
}

impl ProfileLease {
    /// Claim a clean profile without waiting on another session's browser.
    pub fn acquire(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "WebView profile pool must be absolute");
        fs::create_dir_all(root)?;
        for index in 0..MAX_PROFILE_SLOTS {
            let directory = root.join(format!("slot-{index:03}"));
            fs::create_dir_all(&directory)?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(directory.join("lease.lock"))?;
            match lock.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => continue,
                Err(error) => return Err(error.into()),
            }
            if directory.join("in-use").try_exists()? {
                continue;
            }
            fs::write(directory.join("in-use"), uuid::Uuid::new_v4().to_string())?;
            return Ok(Self {
                directory,
                _lock: lock,
            });
        }
        anyhow::bail!("No clean WebView profile slot available; inspect active or quarantined slots")
    }

    /// Only the caller holding confirmed whole-job exit may clear the dirty marker.
    pub fn release_after_job_exit(self) -> Result<()> {
        fs::remove_file(self.directory.join("in-use"))
            .context("Release confirmed WebView profile")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_are_exclusive_reusable_and_quarantined_on_drop() -> Result<()> {
        let root = std::env::temp_dir().join(format!("profile-pool-{}", uuid::Uuid::new_v4()));
        let first = ProfileLease::acquire(&root)?;
        let first_path = first.directory.clone();
        fs::write(first_path.join("cached-data"), b"retain")?;
        let second = ProfileLease::acquire(&root)?;
        assert_ne!(first.directory, second.directory);
        first.release_after_job_exit()?;
        let reused = ProfileLease::acquire(&root)?;
        assert_eq!(reused.directory, first_path);
        assert_eq!(fs::read(reused.directory.join("cached-data"))?, b"retain");
        drop(reused);
        drop(second);
        let replacement = ProfileLease::acquire(&root)?;
        assert_ne!(replacement.directory, first_path);
        assert!(first_path.join("in-use").exists());
        replacement.release_after_job_exit()?;
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
