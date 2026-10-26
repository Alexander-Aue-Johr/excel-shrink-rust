//! Local HTTP hosts used by the Office.js capture clients and Observatory.
//!
//! Each host runs as a child process of the existing CLI. The transport changes
//! neither the saved evidence format nor the lifetime of an Excel comparison.

use anyhow::{bail, Context, Result};
use axum::{
    extract::DefaultBodyLimit,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json, Router,
};
use serde_json::{json, Value};
use std::{ffi::OsString, path::Path};
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;

mod classic;
mod control_room;
mod resident;

/// Use the same Windows build for orchestration and HTTP hosting. WSL callers
/// can select a native companion executable explicitly.
pub fn executable() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("EXCEL_SHRINK_CAPTURE_HOST") {
        let path = std::path::PathBuf::from(path);
        anyhow::ensure!(
            path.is_file(),
            "Capture host executable is missing: {}",
            path.display()
        );
        return Ok(path);
    }
    #[cfg(windows)]
    {
        Ok(std::env::current_exe()?)
    }
    #[cfg(not(windows))]
    {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/release/excel-shrink-cli.exe");
        anyhow::ensure!(
            path.is_file(),
            "Build a Windows CLI or set EXCEL_SHRINK_CAPTURE_HOST for Office capture."
        );
        Ok(path)
    }
}

pub fn run(arguments: Vec<OsString>) -> Result<()> {
    let Some((mode, arguments)) = arguments.split_first() else {
        bail!("Expected capture-host resident, control-room, or classic.");
    };
    // Many comparison groups may have their own host. Two async executor
    // threads per host avoid allocating a thread for every machine CPU.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        match mode.to_str() {
            Some("resident") if arguments.len() == 1 => {
                resident::run(Path::new(&arguments[0])).await
            }
            Some("control-room") if arguments.len() == 1 => {
                control_room::run(Path::new(&arguments[0])).await
            }
            Some("classic") => classic::run(arguments.to_vec()).await,
            _ => bail!("Invalid capture-host mode or settings arguments."),
        }
    })
}

pub(super) type HttpResult<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl ToString) -> Self {
        Self {
            status,
            message: message.to_string(),
        }
    }

    pub fn bad_request(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn conflict(message: impl ToString) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn forbidden(message: impl ToString) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub fn not_found(message: impl ToString) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::bad_request(format!("{error:#}"))
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self::bad_request(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::bad_request(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) async fn read_json(path: &Path) -> Result<Value> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    // Windows PowerShell 5.1 may write a UTF-8 BOM in a settings file.
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    Ok(serde_json::from_slice(bytes).with_context(|| format!("parse {}", path.display()))?)
}

pub(super) async fn write_json(path: &Path, value: &Value, replace: bool) -> Result<()> {
    write_bytes(path, &serde_json::to_vec(value)?, replace).await
}

pub(super) async fn write_bytes(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    publish_bytes(path, bytes, replace, Durability::Buffered).await
}

pub(super) async fn write_durable_bytes(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    publish_bytes(path, bytes, replace, Durability::Disk).await
}

enum Durability {
    Buffered,
    Disk,
}

async fn publish_bytes(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    durability: Durability,
) -> Result<()> {
    let parent = path.parent().context("Output path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let pending = parent.join(format!(".{}.pending", uuid::Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        if matches!(durability, Durability::Disk) {
            file.sync_all().await?;
        }
        drop(file);
        if replace {
            replace_snapshot(&pending, path).await?;
        } else {
            publish_new(&pending, path)?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Remove an unpublished temporary file on error. A completed rename has
    // already removed it; cleanup must not obscure the actual write error.
    let _ = tokio::fs::remove_file(&pending).await;
    result.with_context(|| format!("publish {}", path.display()))
}

async fn replace_snapshot(pending: &Path, destination: &Path) -> std::io::Result<()> {
    // Windows readers outside this host may briefly deny delete sharing. Keep
    // the previous complete snapshot visible while retrying the atomic rename.
    for attempt in 0..20 {
        match tokio::fs::rename(pending, destination).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 19 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the final attempt returns its result")
}

#[cfg(windows)]
fn publish_new(pending: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let from: Vec<u16> = pending.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Flags=0 deliberately refuses to overwrite another request's result.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_new(pending: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::hard_link(pending, destination)
}

pub(super) fn with_web(router: Router, web: &Path, limit: usize) -> Router {
    router
        .fallback_service(ServeDir::new(web))
        .layer(DefaultBodyLimit::max(limit))
        .layer(middleware::from_fn(no_cache))
}

async fn no_cache(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
}

pub(super) async fn serve(router: Router, port: u16) -> Result<()> {
    anyhow::ensure!(port >= 1024, "Invalid localhost port.");
    let ipv4 = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    // Add-in URLs use localhost, which Windows commonly resolves to ::1 first.
    // Listening only on IPv4 adds a connection fallback delay to every poll.
    match tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await {
        Ok(ipv6) => {
            let ipv4_router = router.clone();
            tokio::try_join!(async { axum::serve(ipv4, ipv4_router).await }, async {
                axum::serve(ipv6, router).await
            },)?;
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
            ) =>
        {
            axum::serve(ipv4, router).await?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evidence_cannot_overwrite_a_previously_published_result() -> Result<()> {
        let root = std::env::temp_dir().join(format!("capture-host-{}", uuid::Uuid::new_v4()));
        let path = root.join("result.json");
        write_json(&path, &json!({"first":true}), false).await?;
        assert!(write_json(&path, &json!({"first":false}), false)
            .await
            .is_err());
        assert_eq!(read_json(&path).await?, json!({"first":true}));
        assert_eq!(std::fs::read_dir(&root)?.count(), 1);
        tokio::fs::remove_dir_all(root).await?;
        Ok(())
    }

    #[tokio::test]
    async fn snapshots_replace_complete_json_and_accept_powershell_utf8() -> Result<()> {
        let root = std::env::temp_dir().join(format!("capture-host-{}", uuid::Uuid::new_v4()));
        let path = root.join("state.json");
        write_bytes(&path, b"\xef\xbb\xbf{\"ready\":false}", false).await?;
        assert_eq!(read_json(&path).await?, json!({"ready":false}));
        write_json(&path, &json!({"ready":true}), true).await?;
        assert_eq!(read_json(&path).await?, json!({"ready":true}));
        assert_eq!(std::fs::read_dir(&root)?.count(), 1);
        tokio::fs::remove_dir_all(root).await?;
        Ok(())
    }
}
