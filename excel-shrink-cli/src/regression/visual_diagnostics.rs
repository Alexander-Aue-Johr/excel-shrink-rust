//! Wall-clock diagnostics for regression/Excel automation, including blocked calls.
//! Heartbeats never write to Excel's progress file and never reset its stall timer.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Active {
    name: String,
    detail: String,
    started: Instant,
    detail_started: Instant,
}
struct State {
    next_id: u64,
    active: BTreeMap<u64, Active>,
    timing: Option<File>,
    slow: Option<File>,
    slow_after: Duration,
}
static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();

pub(super) fn seconds_from_env(name: &str, fallback: f64) -> Duration {
    let seconds = std::env::var(name).ok().and_then(|s| s.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.1 && *n <= 86400.0).unwrap_or(fallback);
    Duration::from_secs_f64(seconds)
}
fn stamp() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}
fn open_log(root: &std::path::Path, name: &str) -> Option<File> {
    match OpenOptions::new().create_new(true).write(true).open(root.join(name)) {
        Ok(file) => Some(file),
        Err(error) => { eprintln!("Cannot create visual diagnostics {}: {error}", root.join(name).display()); None }
    }
}
fn state() -> &'static Arc<Mutex<State>> {
    STATE.get_or_init(|| {
        let root = std::env::var_os("EXCEL_SHRINK_VISUAL_LOG_DIR").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("logs"));
        if let Err(error) = fs::create_dir_all(&root) {
            eprintln!("Cannot create visual log directory {}: {error}", root.display());
        }
        let suffix = format!("{}-{}", stamp(), std::process::id());
        let timing_name = format!("visual-timing-{suffix}.jsonl");
        let slow_name = format!("visual-slow-{suffix}.jsonl");
        let timing = open_log(&root, &timing_name);
        let slow = open_log(&root, &slow_name);
        eprintln!("Visual timings: {}\nVisual slow phases: {}", root.join(timing_name).display(), root.join(slow_name).display());
        let shared = Arc::new(Mutex::new(State {
            next_id: 1, active: BTreeMap::new(), timing, slow,
            slow_after: seconds_from_env("EXCEL_SHRINK_VISUAL_SLOW_SECONDS", 10.0),
        }));
        let worker = Arc::clone(&shared);
        let interval = seconds_from_env("EXCEL_SHRINK_VISUAL_HEARTBEAT_SECONDS", 5.0);
        if let Err(error) = std::thread::Builder::new().name("visual-watchdog".into()).spawn(move || loop {
            std::thread::sleep(interval);
            let Ok(mut state) = worker.lock() else { break };
            let rows: Vec<_> = state.active.iter().map(|(id, p)| {
                (*id, p.name.clone(), p.detail.clone(), p.started.elapsed(), p.detail_started.elapsed())
            }).collect();
            for (id, name, detail, elapsed, detail_elapsed) in rows {
                record(&mut state, "heartbeat", id, &name, &detail, elapsed, detail_elapsed, "running");
            }
        }) { eprintln!("Cannot start visual watchdog: {error}"); }
        shared
    })
}
fn write_json(file: &mut Option<File>, row: &serde_json::Value) {
    if let Some(output) = file {
        let result = serde_json::to_writer(&mut *output, row)
            .map_err(std::io::Error::other)
            .and_then(|_| output.write_all(b"\n"))
            .and_then(|_| output.flush());
        if let Err(error) = result {
            eprintln!("Visual diagnostics write failed: {error}");
            *file = None; // Report once, do not turn a full disk into a log storm.
        }
    }
}
fn record(state: &mut State, event: &str, id: u64, name: &str, detail: &str,
          elapsed: Duration, detail_elapsed: Duration, status: &str) {
    let slow = elapsed >= state.slow_after;
    let row = serde_json::json!({"timestamp_unix_ms":stamp(), "pid":std::process::id(),
        "event":event, "phase_id":id, "phase":name, "detail":detail,
        "elapsed_ms":elapsed.as_millis(), "detail_elapsed_ms":detail_elapsed.as_millis(),
        "status":status, "slow":slow, "timing":"wall time; nested phases overlap"});
    write_json(&mut state.timing, &row);
    // Slow logs are a ranking of finished operations, not repeated "still running"
    // messages. Heartbeats remain in the full log and never reset a COM timeout.
    if should_write_slow(event, elapsed, state.slow_after) { write_json(&mut state.slow, &row); }
    // Heartbeats stay in visual-timing JSONL only. They are liveness data, not
    // user-facing progress messages and must not flood the normal run log.
    if event == "heartbeat" { return; }
    let message = format!("[visual:{event}] {name}: {detail} elapsed={:.1}s current_step={:.1}s status={status}",
        elapsed.as_secs_f64(), detail_elapsed.as_secs_f64());
    if slow {
        tracing::warn!(event = "slow_visual_phase", phase_id = id, phase = name,
            elapsed_ms = elapsed.as_millis() as u64, "{message}");
        if std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
            eprintln!("\x1b[31m{message}\x1b[0m");
        }
    } else if event == "progress" {
        tracing::debug!(event = "visual_progress", phase_id = id, "{message}");
    } else {
        tracing::info!(event = "visual_phase", phase_id = id, "{message}");
    }
}

pub(super) struct Phase { id: u64, state: Arc<Mutex<State>> }
pub(super) fn phase(name: impl Into<String>, detail: impl Into<String>) -> Phase {
    let shared = Arc::clone(state());
    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
    let id = state.next_id;
    state.next_id += 1;
    let name = name.into(); let detail = detail.into();
    record(&mut state, "start", id, &name, &detail, Duration::ZERO, Duration::ZERO, "running");
    state.active.insert(id, Active { name, detail, started:Instant::now(), detail_started:Instant::now() });
    drop(state);
    Phase { id, state:shared }
}
impl Phase {
    /// Update the currently displayed/diagnostic detail without creating a progress
    /// event. Machine heartbeats use this so liveness does not become log spam.
    pub(super) fn set_detail_silent(&self, detail: impl Into<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.get_mut(&self.id) {
            active.detail = detail.into();
            active.detail_started = Instant::now();
        }
    }

    /// Ends the previous reported step before tracking the next one. A progress gap
    /// is not attributed to CPU work: it may include COM, disk or scheduler waits.
    pub(super) fn detail(&self, detail: impl Into<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut active) = state.active.remove(&self.id) {
            // Batched progress lines can arrive together. Do not write thousands
            // of synthetic zero-duration steps; script stopwatches give real totals.
            let elapsed = active.detail_started.elapsed();
            if elapsed >= Duration::from_millis(100) {
                record(&mut state, "progress", self.id, &active.name, &active.detail,
                    elapsed, elapsed, "observed_progress_gap");
            }
            active.detail = detail.into(); active.detail_started = Instant::now();
            state.active.insert(self.id, active);
        }
    }
}
impl Drop for Phase {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else { return };
        if let Some(active) = state.active.remove(&self.id) {
            record(&mut state, "end", self.id, &active.name, &active.detail,
                active.started.elapsed(), active.detail_started.elapsed(),
                if std::thread::panicking() { "unwinding" } else { "scope_ended" });
        }
    }
}

fn should_write_slow(event: &str, elapsed: Duration, threshold: Duration) -> bool {
    matches!(event, "end" | "progress") && elapsed >= threshold
}
#[cfg(test)]
mod visual_diagnostics_tests {
    use super::*;
    #[test]
    fn slow_file_excludes_repeated_heartbeats() {
        let threshold = Duration::from_secs(10);
        assert!(!should_write_slow("heartbeat", Duration::from_secs(999), threshold));
        assert!(should_write_slow("end", threshold, threshold));
        assert!(should_write_slow("progress", threshold, threshold));
        assert!(!should_write_slow("end", Duration::from_secs(9), threshold));
    }
}
