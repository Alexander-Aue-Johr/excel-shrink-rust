//! Small, bounded observations from the process doing the work. Never a watchdog.
use super::*;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct Monitor {
    path: PathBuf,
    value: Value,
}
static MONITOR: OnceLock<Mutex<Monitor>> = OnceLock::new();

pub(super) fn initialize(job: &MemberJob, record: &Value) {
    let Some(root) = &job.control_room else {
        return;
    };
    let directory = root
        .join("groups")
        .join(&artifact_cache::sha256(job.identity.as_bytes())[..20]);
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let value = json!({"attempt_id":record["control_room_attempt"]["id"],"pid":std::process::id(),
        "process_started_unix_ms":process_started_unix_ms(),"role":"workbook_coordinator",
        "workbook":job.name,"phase":"starting","state":"starting","state_since_unix_ms":now(),"events":[]});
    let _ = MONITOR.set(Mutex::new(Monitor {
        path: directory.join("coordinator.json"),
        value,
    }));
    event(
        "starting",
        "worker_started",
        "Workbook worker started",
        false,
        None,
        json!({"mode":job.mode}),
    );
}

pub(in crate::regression) fn event(
    phase: &str,
    kind: &str,
    message: &str,
    progress: bool,
    wait: Option<&str>,
    details: Value,
) {
    let Some(monitor) = MONITOR.get() else {
        return;
    };
    let Ok(mut monitor) = monitor.lock() else {
        return;
    };
    observe(
        &mut monitor.value,
        now(),
        phase,
        kind,
        message,
        progress,
        wait,
        details,
    );
    if let Ok(bytes) = serde_json::to_vec(&monitor.value) {
        let _ = artifact_cache::atomic_write(&monitor.path, &bytes);
    }
}

fn observe(
    value: &mut Value,
    time: u64,
    phase: &str,
    kind: &str,
    message: &str,
    progress: bool,
    wait: Option<&str>,
    details: Value,
) {
    if value["phase"] != phase || value["wait_reason"] != json!(wait) {
        value["state_since_unix_ms"] = json!(time);
    }
    value["phase"] = json!(phase);
    value["state"] = json!(phase);
    value["wait_reason"] = json!(wait);
    let event = json!({"sequence":value["last_event"]["sequence"].as_u64().unwrap_or(0)+1,
        "at_unix_ms":time,"kind":kind,"message":message,"details":details});
    value["last_event"] = event.clone();
    if progress {
        value["last_progress_unix_ms"] = json!(time);
        value["last_success"] = event.clone();
    }
    let history = value["events"].as_array_mut().unwrap();
    history.push(event);
    if history.len() > 24 {
        history.remove(0);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(windows)]
pub(super) fn process_started_unix_ms() -> Option<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            created: *mut u64,
            exited: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
    }
    let (mut created, mut exited, mut kernel, mut user) = (0, 0, 0, 0);
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (ok != 0)
        .then_some(created)?
        .checked_sub(116_444_736_000_000_000)
        .map(|ticks| ticks / 10_000)
}
#[cfg(not(windows))]
pub(super) fn process_started_unix_ms() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waiting_does_not_fabricate_progress_or_reset_state_age() {
        let mut value = json!({"phase":"start","events":[]});
        observe(
            &mut value,
            100,
            "capturing",
            "received",
            "Image received",
            true,
            None,
            json!({}),
        );
        observe(
            &mut value,
            200,
            "waiting",
            "waiting",
            "Waiting for Excel",
            false,
            Some("excel"),
            json!({}),
        );
        observe(
            &mut value,
            300,
            "waiting",
            "waiting",
            "Still waiting",
            false,
            Some("excel"),
            json!({}),
        );
        assert_eq!(value["last_progress_unix_ms"], 100);
        assert_eq!(value["state_since_unix_ms"], 200);
        for time in 400..500 {
            observe(
                &mut value,
                time,
                "waiting",
                "waiting",
                "Still waiting",
                false,
                Some("excel"),
                json!({}),
            );
        }
        assert_eq!(value["events"].as_array().unwrap().len(), 24);
        assert_eq!(value["last_success"]["at_unix_ms"], 100);
    }
}
