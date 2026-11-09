//! Shared hashes and durable same-directory file publication.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut result = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut result, "{b:02x}").unwrap();
    }
    result
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    Ok(hex(&hash.finalize()))
}
pub fn unique_path(parent: &Path, prefix: &str) -> Result<PathBuf> {
    fs::create_dir_all(parent)?;
    Ok(parent.join(format!(
        ".{prefix}-{}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("output has no parent")?;
    let pending = unique_path(parent, "pending")?;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&pending, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result.with_context(|| format!("atomically write {}", path.display()))
}
pub fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    #[cfg(not(windows))]
    fs::rename(source, target)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let from = fs::canonicalize(source)?;
        let to = fs::canonicalize(target.parent().context("no parent")?)?
            .join(target.file_name().context("no filename")?);
        let a: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0x1 | 0x8) } != 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            // Readers without FILE_SHARE_DELETE (including PowerShell) can block
            // replacement briefly. Keep the old complete file until they close;
            // never delete it to work around a sharing or permission error.
            let transient = matches!(error.raw_os_error(), Some(5 | 32 | 33));
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if !transient || remaining.is_zero() {
                return Err(error).with_context(|| {
                    format!("replace {} with {}", target.display(), source.display())
                });
            }
            std::thread::sleep(remaining.min(std::time::Duration::from_millis(50)));
        }
    }
    Ok(())
}
