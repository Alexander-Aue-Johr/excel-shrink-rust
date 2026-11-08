//! Loopback-only exclusive leases for run-owned native workbook workers.
use super::*;
use crate::capture_host::{ApiError, HttpResult};
use axum::{
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoolSettings {
    worker: Settings,
    workers: usize,
    token: String,
    ready_path: PathBuf,
    port: u16,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Owner {
    pid: i64,
    started_ms: i64,
    comparison_id: String,
}

struct Lease {
    token: String,
    owner: Owner,
    opened: bool,
}

struct Slot {
    worker: Option<Worker>,
    identity: Value,
    lease: Option<Lease>,
    cleanup_error: Option<String>,
    manifest_id: String,
    capture_port: u16,
    origin_reservation: Option<std::net::TcpListener>,
    last_released_token: Option<String>,
}

struct Pool {
    settings: Settings,
    token: String,
    slots: Vec<Mutex<Slot>>,
    admissions: Mutex<HashMap<String, usize>>,
}

type Reply = HttpResult<Json<Value>>;

fn authenticate(headers: &HeaderMap, token: &str, name: &str) -> HttpResult<()> {
    if headers.get(name).and_then(|value| value.to_str().ok()) != Some(token) {
        return Err(ApiError::forbidden("Invalid workbook pool credential"));
    }
    Ok(())
}

fn live(owner: &Owner) -> bool {
    crate::native_process::process_alive(owner.pid, owner.started_ms) == Some(true)
}

fn reserve_memory() -> Result<()> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    memory.dwLength = std::mem::size_of_val(&memory) as u32;
    ensure!(
        unsafe { GlobalMemoryStatusEx(&mut memory) } != 0,
        "Cannot observe workbook pool RAM reserve"
    );
    ensure!(
        memory.ullAvailPhys >= 512 * 1024 * 1024 && memory.ullAvailPageFile >= 512 * 1024 * 1024,
        "Workbook pool requires 512 MiB physical and commit reserve"
    );
    Ok(())
}

fn start_slot(settings: &Settings) -> Result<(Worker, Value)> {
    reserve_memory()?;
    let directory = settings.profile_root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&directory)?;
    let mut settings = settings.clone();
    fs::copy(&settings.seed, directory.join("seed.xlsx"))?;
    settings.seed = directory.join("seed.xlsx");
    settings.profile_root = directory.join("profile");
    let mut worker = Worker::start(&settings)?;
    let identity = worker.initialize(Duration::from_secs(settings.timeout_seconds))?;
    reserve_memory()?;
    Ok((worker, identity))
}

fn retire(slot: &mut Slot) {
    if let Some(mut worker) = slot.worker.take() {
        if let Err(error) = worker.stop() {
            slot.cleanup_error = Some(format!("{error:#}"));
        }
    }
}

fn lease_reply(index: usize, slot: &Slot) -> Value {
    let lease = slot.lease.as_ref().expect("lease just assigned");
    json!({"slot":index,"lease_token":lease.token,"comparison_id":lease.owner.comparison_id,
        "process":slot.identity,"manifest_id":slot.manifest_id,"capture_port":slot.capture_port})
}

fn assign(index: usize, slot: &mut Slot, owner: Owner) -> Value {
    drop(slot.origin_reservation.take());
    slot.lease = Some(Lease {
        token: uuid::Uuid::new_v4().to_string(),
        owner,
        opened: false,
    });
    lease_reply(index, slot)
}

async fn acquire(
    State(pool): State<Arc<Pool>>,
    headers: HeaderMap,
    Json(owner): Json<Owner>,
) -> Reply {
    authenticate(&headers, &pool.token, "X-Pool-Token")?;
    uuid::Uuid::parse_str(&owner.comparison_id).map_err(ApiError::bad_request)?;
    if !live(&owner) {
        return Err(ApiError::conflict("Comparison owner identity is not live"));
    }
    let value = tokio::task::spawn_blocking(move || -> HttpResult<Value> {
        let mut admissions = pool
            .admissions
            .try_lock()
            .map_err(|_| ApiError::conflict("Workbook pool admission is busy"))?;
        if let Some(index) = admissions.get(&owner.comparison_id) {
            let slot = pool.slots[*index]
                .try_lock()
                .map_err(|_| ApiError::conflict("Existing workbook lease is busy"))?;
            let lease = slot
                .lease
                .as_ref()
                .ok_or_else(|| ApiError::conflict("Worker lease is being retired"))?;
            if lease.owner != owner {
                return Err(ApiError::conflict("Lease request owner differs"));
            }
            return Ok(lease_reply(*index, &slot));
        }
        // Prefer already warm slots before attempting replacement of a retired worker.
        for (index, shared) in pool.slots.iter().enumerate() {
            if let Ok(mut slot) = shared.try_lock() {
                if slot.lease.is_none() && slot.worker.is_some() && slot.cleanup_error.is_none() {
                    let identity = slot.worker.as_ref().and_then(|worker| worker.identity);
                    match identity.and_then(|(pid, started)| {
                        crate::native_process::process_alive(pid, started)
                    }) {
                        Some(true) => {
                            admissions.insert(owner.comparison_id.clone(), index);
                            return Ok(assign(index, &mut slot, owner));
                        }
                        Some(false) => retire(&mut slot),
                        None => {
                            slot.cleanup_error = Some("Idle worker liveness is unconfirmed".into())
                        }
                    }
                }
            }
        }
        for (index, shared) in pool.slots.iter().enumerate() {
            if let Ok(mut slot) = shared.try_lock() {
                if slot.lease.is_none() && slot.worker.is_none() && slot.cleanup_error.is_none() {
                    let (worker, identity) = start_slot(&pool.settings)?;
                    slot.worker = Some(worker);
                    slot.identity = identity;
                    admissions.insert(owner.comparison_id.clone(), index);
                    return Ok(assign(index, &mut slot, owner));
                }
            }
        }
        Err(ApiError::conflict(
            "No workbook worker is currently available",
        ))
    })
    .await
    .context("Workbook pool admission task")??;
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_authorization_requires_current_token_and_live_birth_identity() -> Result<()> {
        let reservation = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let mut slot = Slot {
            worker: None,
            identity: Value::Null,
            cleanup_error: None,
            manifest_id: uuid::Uuid::new_v4().to_string(),
            capture_port: reservation.local_addr()?.port(),
            origin_reservation: Some(reservation),
            last_released_token: None,
            lease: Some(Lease {
                token: "current-token".into(),
                opened: false,
                owner: Owner {
                    pid: i64::from(std::process::id()),
                    started_ms: crate::native_process::current_started_unix_ms().unwrap(),
                    comparison_id: uuid::Uuid::new_v4().to_string(),
                },
            }),
        };
        let mut headers = HeaderMap::new();
        headers.insert("X-Lease-Token", "old-token".parse()?);
        assert!(authorized_lease(&mut slot, &headers).is_err());
        headers.insert("X-Lease-Token", "current-token".parse()?);
        assert!(authorized_lease(&mut slot, &headers).is_ok());
        slot.lease.as_mut().unwrap().owner.started_ms -= 1000;
        assert!(authorized_lease(&mut slot, &headers).is_err());
        slot.lease = None;
        assert!(authorized_lease(&mut slot, &headers).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn release_needs_a_token_and_retries_do_not_change_a_new_lease() -> Result<()> {
        use axum::response::IntoResponse;
        let reservation = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let port = reservation.local_addr()?.port();
        drop(reservation);
        let owner = Owner {
            pid: i64::from(std::process::id()),
            started_ms: crate::native_process::current_started_unix_ms().unwrap(),
            comparison_id: uuid::Uuid::new_v4().to_string(),
        };
        let pool = Arc::new(Pool {
            settings: Settings {
                seed: PathBuf::new(),
                excel_executable: PathBuf::new(),
                capture_root: PathBuf::new(),
                profile_root: PathBuf::new(),
                timeout_seconds: 1,
            },
            token: "pool-token".into(),
            admissions: Mutex::new(HashMap::from([(owner.comparison_id.clone(), 0)])),
            slots: vec![Mutex::new(Slot {
                worker: None,
                identity: Value::Null,
                lease: Some(Lease {
                    token: "lease-token".into(),
                    owner: owner.clone(),
                    opened: false,
                }),
                cleanup_error: None,
                manifest_id: uuid::Uuid::new_v4().to_string(),
                capture_port: port,
                origin_reservation: None,
                last_released_token: None,
            })],
        });
        let mut headers = HeaderMap::new();
        headers.insert("X-Pool-Token", "pool-token".parse()?);
        let failure = release(State(pool.clone()), RoutePath(0), headers.clone())
            .await
            .unwrap_err();
        assert_eq!(
            failure.into_response().status(),
            axum::http::StatusCode::FORBIDDEN
        );
        headers.insert("X-Lease-Token", "lease-token".parse()?);
        assert_eq!(
            release(State(pool.clone()), RoutePath(0), headers.clone())
                .await
                .unwrap()
                .0["ok"],
            true
        );
        {
            let mut slot = pool.slots[0].lock().unwrap();
            assign(
                0,
                &mut slot,
                Owner {
                    comparison_id: uuid::Uuid::new_v4().to_string(),
                    ..owner
                },
            );
        }
        assert_eq!(
            release(State(pool.clone()), RoutePath(0), headers)
                .await
                .unwrap()
                .0["repeated"],
            true
        );
        assert!(pool.slots[0].lock().unwrap().lease.is_some());
        Ok(())
    }
}

fn authorized_lease<'a>(slot: &'a mut Slot, headers: &HeaderMap) -> HttpResult<&'a mut Lease> {
    let lease = slot
        .lease
        .as_mut()
        .ok_or_else(|| ApiError::conflict("Worker lease is retired"))?;
    authenticate(headers, &lease.token, "X-Lease-Token")?;
    if !live(&lease.owner) {
        return Err(ApiError::conflict("Comparison owner identity is not live"));
    }
    Ok(lease)
}

async fn command(
    State(pool): State<Arc<Pool>>,
    RoutePath(index): RoutePath<usize>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Reply {
    authenticate(&headers, &pool.token, "X-Pool-Token")?;
    if index >= pool.slots.len() {
        return Err(ApiError::not_found("Unknown workbook slot"));
    }
    let result = tokio::task::spawn_blocking(move || -> HttpResult<Value> {
        let mut slot = pool.slots[index]
            .try_lock()
            .map_err(|_| ApiError::conflict("Workbook worker is busy"))?;
        let lease = authorized_lease(&mut slot, &headers)?;
        let kind = request["kind"]
            .as_str()
            .ok_or_else(|| ApiError::bad_request("Missing workbook command kind"))?;
        if !matches!(kind, "open" | "close" | "save_copy" | "rebuild") {
            return Err(ApiError::bad_request("Unsupported leased workbook command"));
        }
        if (kind == "open" && lease.opened)
            || (matches!(kind, "save_copy" | "rebuild") && !lease.opened)
        {
            return Err(ApiError::conflict(
                "Workbook command does not match the lease state",
            ));
        }
        let worker = slot
            .worker
            .as_mut()
            .ok_or_else(|| ApiError::conflict("Workbook worker was retired"))?;
        let reply = worker.command(&request, Duration::from_secs(pool.settings.timeout_seconds));
        match reply {
            Ok(value) if value["ok"] == true => {
                if kind == "open" {
                    slot.lease.as_mut().unwrap().opened = true;
                }
                if kind == "close" {
                    slot.lease.as_mut().unwrap().opened = false;
                }
                Ok(value)
            }
            Ok(value) => {
                retire(&mut slot);
                Ok(value)
            }
            Err(error) => {
                retire(&mut slot);
                Err(error.into())
            }
        }
    })
    .await
    .context("Workbook pool command task")??;
    Ok(Json(result))
}

async fn release(
    State(pool): State<Arc<Pool>>,
    RoutePath(index): RoutePath<usize>,
    headers: HeaderMap,
) -> Reply {
    authenticate(&headers, &pool.token, "X-Pool-Token")?;
    let shared = pool
        .slots
        .get(index)
        .ok_or_else(|| ApiError::not_found("Unknown workbook slot"))?;
    let mut slot = shared
        .try_lock()
        .map_err(|_| ApiError::conflict("Workbook worker is busy"))?;
    if slot.last_released_token.as_ref().is_some_and(|token| {
        headers
            .get("X-Lease-Token")
            .and_then(|value| value.to_str().ok())
            == Some(token.as_str())
    }) {
        return Ok(Json(json!({"ok":true,"repeated":true})));
    }
    let opened = authorized_lease(&mut slot, &headers)?.opened;
    if slot.cleanup_error.is_some() || (opened && slot.worker.is_some()) {
        return Err(ApiError::conflict(
            "Workbook closure or cleanup is unconfirmed",
        ));
    }
    let mut admissions = pool
        .admissions
        .try_lock()
        .map_err(|_| ApiError::conflict("Workbook pool admission is busy"))?;
    slot.origin_reservation = Some(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, slot.capture_port)).map_err(
            |_| ApiError::conflict("Comparison capture host must stop before lease release"),
        )?,
    );
    let lease = slot.lease.take().unwrap();
    admissions.remove(&lease.owner.comparison_id);
    slot.last_released_token = Some(lease.token);
    Ok(Json(json!({"ok":true})))
}

async fn health(State(pool): State<Arc<Pool>>, headers: HeaderMap) -> Reply {
    authenticate(&headers, &pool.token, "X-Pool-Token")?;
    let workers: Vec<_> = pool
        .slots
        .iter()
        .enumerate()
        .map(|(index, shared)| match shared.try_lock() {
            Ok(slot) => {
                let alive = slot
                    .worker
                    .as_ref()
                    .and_then(|worker| worker.identity)
                    .and_then(|(pid, started)| crate::native_process::process_alive(pid, started));
                let ready = alive == Some(true) && slot.cleanup_error.is_none();
                json!({"slot":index,"leased":slot.lease.is_some(),"ready":ready,
                "available":ready && slot.lease.is_none(),"process_alive":alive,
                "cleanup_error":slot.cleanup_error,"process":slot.identity})
            }
            Err(_) => json!({"slot":index,"busy":true}),
        })
        .collect();
    Ok(Json(json!({"workers":workers})))
}

/// Start a run-owned loopback service. The corpus coordinator owns this process.
pub(crate) async fn run(path: &Path) -> Result<()> {
    let settings: PoolSettings = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        (1..=64).contains(&settings.workers),
        "Workbook pool size must be 1 through 64"
    );
    uuid::Uuid::parse_str(&settings.token)?;
    ensure!(
        settings.ready_path.is_absolute() && settings.worker.profile_root.is_absolute(),
        "Pool paths must be absolute"
    );
    fs::create_dir(&settings.worker.profile_root)?;
    let mut starts = Vec::new();
    for _ in 0..settings.workers {
        let worker_settings = settings.worker.clone();
        starts.push(tokio::task::spawn_blocking(move || {
            start_slot(&worker_settings)
        }));
    }
    let mut ready_workers = Vec::new();
    let mut failure = None;
    for start in starts {
        match start
            .await
            .context("Start workbook pool member")
            .and_then(|result| result)
        {
            Ok(worker) => ready_workers.push(worker),
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    let mut slots = Vec::new();
    for (worker, identity) in ready_workers {
        let probe = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let capture_port = probe.local_addr()?.port();
        slots.push(Mutex::new(Slot {
            worker: Some(worker),
            identity,
            lease: None,
            cleanup_error: None,
            manifest_id: uuid::Uuid::new_v4().to_string(),
            capture_port,
            origin_reservation: Some(probe),
            last_released_token: None,
        }));
    }
    let pool = Arc::new(Pool {
        settings: settings.worker,
        token: settings.token,
        slots,
        admissions: Mutex::new(HashMap::new()),
    });
    let listener =
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, settings.port)).await?;
    let address = listener.local_addr()?;
    crate::capture_host::write_json(&settings.ready_path, &json!({
        "url":format!("http://localhost:{}",address.port()),"workers":pool.slots.len(),
        "owner_pid":std::process::id(),"owner_started_ms":crate::native_process::current_started_unix_ms()
    }), false).await?;
    let maintenance = pool.clone();
    let maintenance_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let pool = maintenance.clone();
            let _ = tokio::task::spawn_blocking(move || {
                for shared in &pool.slots {
                    if let Ok(mut slot) = shared.try_lock() {
                        if slot.lease.as_ref().is_some_and(|lease| {
                            crate::native_process::process_alive(
                                lease.owner.pid,
                                lease.owner.started_ms,
                            ) == Some(false)
                        }) {
                            let Ok(mut admissions) = pool.admissions.try_lock() else {
                                continue;
                            };
                            retire(&mut slot);
                            match std::net::TcpListener::bind((
                                std::net::Ipv4Addr::LOCALHOST,
                                slot.capture_port,
                            )) {
                                Ok(reservation) => slot.origin_reservation = Some(reservation),
                                Err(error) => {
                                    slot.cleanup_error =
                                        Some(format!("Capture origin cleanup unconfirmed: {error}"))
                                }
                            }
                            let lease = slot.lease.take().unwrap();
                            admissions.remove(&lease.owner.comparison_id);
                            if slot.cleanup_error.is_none() {
                                slot.last_released_token = Some(lease.token);
                            }
                        }
                    }
                }
            })
            .await;
        }
    });
    let router = Router::new()
        .route("/health", get(health))
        .route("/acquire", post(acquire))
        .route("/command/{index}", post(command))
        .route("/release/{index}", post(release))
        .layer(DefaultBodyLimit::max(MAX_FRAME_BYTES))
        .with_state(pool);
    let result = axum::serve(listener, router).await;
    maintenance_task.abort();
    result?;
    Ok(())
}
