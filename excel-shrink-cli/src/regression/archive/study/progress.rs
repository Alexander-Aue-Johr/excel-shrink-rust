//! Assignment progress derived from unique workbook records, never profile counts.
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const WINDOW: f64 = 900.0;
static RUNS: OnceLock<Mutex<BTreeMap<PathBuf, Tracker>>> = OnceLock::new();

#[derive(Clone, Copy, Default)]
struct State {
    completed: bool,
    skipped: bool,
    prepared: bool,
    diagnosis_failed: bool,
}

impl State {
    fn from_record(record: &Value) -> Self {
        let completed = record["complete"] == true
            && matches!(record["status"].as_str(), Some("unchanged" | "differences"))
            && !whitespace::needed(record);
        Self {
            completed,
            skipped: record["complete"] == true && record["status"] == "skipped",
            prepared: record["prepared"] == true || completed,
            diagnosis_failed: record["whitespace_diagnosis"]["status"] == "failed",
        }
    }
}

#[derive(Default)]
struct Rate {
    seconds: f64,
    seen: HashSet<String>,
    recent: VecDeque<f64>,
    finished: usize,
}

impl Rate {
    fn record(&mut self, id: &str) {
        if self.seen.insert(id.to_owned()) {
            self.recent.push_back(self.seconds);
            self.finished += 1;
        }
    }
    fn advance(&mut self, elapsed: f64) {
        self.seconds += elapsed;
        while self
            .recent
            .front()
            .is_some_and(|time| *time < self.seconds - WINDOW)
        {
            self.recent.pop_front();
        }
    }
    fn per_hour(&self) -> Option<f64> {
        (self.seconds > 0.0).then(|| self.recent.len() as f64 * 3600.0 / self.seconds.min(WINDOW))
    }
    fn estimate(&self, remaining: usize) -> Option<f64> {
        if remaining == 0 {
            return Some(0.0);
        }
        (self.recent.len() >= 3 && self.seconds >= 30.0)
            .then(|| remaining as f64 * 3600.0 / self.per_hour().unwrap())
    }
}

struct Tracker {
    run_id: Value,
    scope: Value,
    rows: BTreeMap<String, State>,
    active: BTreeMap<String, JobMode>,
    preparation: Rate,
    comparison: Rate,
    attempts: AttemptActivity,
    phase: String,
    last: Instant,
    active_seconds: f64,
    target: usize,
    captures_paused: bool,
    limited: bool,
    finished: bool,
}

#[derive(Default)]
struct AttemptActivity {
    started: usize,
    compared: usize,
    skipped: usize,
    interrupted: usize,
    retried: usize,
    rate: Rate,
    outcomes: VecDeque<Value>,
}

struct Attempt {
    sequence: usize,
    retried: bool,
    workbook: String,
}

impl Tracker {
    fn start_attempt(&mut self, record: &Value) -> Attempt {
        self.attempts.started += 1;
        Attempt {
            sequence: self.attempts.started,
            // Admission clears `complete` before launching the coordinator.
            // The previous outcome and execution still identify a retry here.
            retried: matches!(
                record["status"].as_str(),
                Some("skipped" | "unchanged" | "differences")
            ) || record["last_execution"]["mode"] == "Compare",
            workbook: record["name"].as_str().unwrap_or("Unknown workbook").into(),
        }
    }

    fn finish_attempt(&mut self, attempt: Attempt, record: &Value, now_ms: u64) {
        let status = match record["status"].as_str() {
            Some("unchanged" | "differences") if record["complete"] == true => {
                self.attempts.compared += 1;
                record["status"].as_str().unwrap()
            }
            Some("skipped") if record["complete"] == true => {
                self.attempts.skipped += 1;
                "skipped"
            }
            _ => {
                self.attempts.interrupted += 1;
                "interrupted"
            }
        };
        self.attempts.retried += usize::from(attempt.retried);
        self.attempts.rate.record(&attempt.sequence.to_string());
        let reason: String = record["error"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(600)
            .collect();
        self.attempts.outcomes.push_front(json!({
            "workbook": attempt.workbook, "status": status, "reason": reason,
            "retried": attempt.retried, "at_unix_ms": now_ms,
            "group_id": control_room::group_id(record["identity"].as_str().unwrap_or("")),
            "attempt_id": record["control_room_attempt"]["id"],
        }));
        self.attempts.outcomes.truncate(5);
    }

    fn observe(&mut self, record: &Value) {
        let Some(id) = record["identity"].as_str() else {
            return;
        };
        if !self.rows.contains_key(id) {
            return;
        }
        let state = State::from_record(record);
        if state.completed {
            self.comparison.record(id);
        }
        if state.prepared {
            self.preparation.record(id);
        }
        self.rows.insert(id.to_owned(), state);
    }
    fn paused(&self) -> bool {
        !self.finished
            && ((self.target == 0 && self.active.is_empty())
                || (self.captures_paused && self.phase != "preparing"))
    }
    fn advance(&mut self, seconds: f64) {
        // Capture pause only prevents new jobs; already dispatched jobs may
        // still finish. Keep their wall time in the denominator. Only a fully
        // drained workbook pause proves that no work is being performed.
        if self.finished || (self.target == 0 && self.active.is_empty()) {
            return;
        }
        self.active_seconds += seconds;
        if self.phase == "preparing" {
            self.preparation.advance(seconds);
        } else {
            self.comparison.advance(seconds);
            self.attempts.rate.advance(seconds);
        }
    }
    fn snapshot(&self, now_ms: u64) -> Value {
        let total = self.rows.len();
        let completed = self.rows.values().filter(|row| row.completed).count();
        let skipped = self.rows.values().filter(|row| row.skipped).count();
        let remaining = total - completed - skipped;
        let prepared = self.rows.values().filter(|row| row.prepared).count();
        let preparation_remaining = self
            .rows
            .values()
            .filter(|row| !row.prepared && !row.skipped)
            .count();
        let phase = if self.finished {
            &self.phase
        } else if self.target == 0 {
            if self.active.is_empty() {
                "paused"
            } else {
                "draining"
            }
        } else {
            &self.phase
        };
        let paused = self.paused();
        let preparation_eta = self.preparation.estimate(preparation_remaining);
        let comparison_eta = self.comparison.estimate(remaining);
        let (eta_status, eta) = if remaining == 0 {
            ("complete", Some(0.0))
        } else if paused {
            ("paused", None)
        } else if self.limited {
            ("limited", None)
        } else if self.finished {
            ("unavailable", None)
        } else if preparation_remaining > 0 && preparation_eta.is_none() {
            ("preparation_pending", None)
        } else if let (Some(preparation), Some(comparison)) = (preparation_eta, comparison_eta) {
            ("estimating", Some(preparation + comparison))
        } else {
            ("insufficient_samples", None)
        };
        let rate = self.comparison.per_hour();
        json!({"version":1,"run_id":self.run_id,"scope":self.scope,"phase":phase,"paused":paused,
            "updated_unix_ms":now_ms,
            "counts":{"total":total,"completed":completed,"skipped":skipped,"remaining":remaining,
                "in_progress":self.active.len(),"prepared":prepared,"preparation_remaining":preparation_remaining},
            "diagnosis_failed":self.rows.values().filter(|row| row.diagnosis_failed).count(),
            "rates":{"window_seconds":WINDOW,"completed_per_hour":rate,"completed_per_minute":rate.map(|rate|rate/60.0),
                "average_completed_per_hour":if self.active_seconds>0.0 {Some(self.comparison.finished as f64*3600.0/self.active_seconds)} else {None},
                "preparation_per_hour":self.preparation.per_hour(),"samples":self.comparison.recent.len(),
                "active_seconds":self.active_seconds,"comparison_active_seconds":self.comparison.seconds,
                "preparation_active_seconds":self.preparation.seconds,"new_completions":self.comparison.finished},
            "activity":{"source":"runner","started_attempts":self.attempts.started,"finished_attempts":self.attempts.rate.finished,
                "compared_attempts":self.attempts.compared,"skipped_attempts":self.attempts.skipped,
                "interrupted_attempts":self.attempts.interrupted,"retried_attempts":self.attempts.retried,
                "attempts_per_hour":self.attempts.rate.per_hour(),"recent_outcomes":self.attempts.outcomes},
            "eta":{"status":eta_status,"remaining_seconds":eta,
                "finish_unix_ms":eta.filter(|_| !paused).map(|seconds|now_ms.saturating_add((seconds*1000.0) as u64)),
                "scope":"selected_workbooks"}})
    }
}

pub(super) fn initialize(
    output: &Path,
    sources: &[Source],
    records: &[Value],
    run: &Value,
) -> Result<()> {
    let rows = sources
        .iter()
        .flat_map(|source| {
            source
                .members
                .iter()
                .filter(|(index, _)| source.selection.contains(*index))
                .map(|(index, name)| (identity(source, *index, name), State::default()))
        })
        .collect();
    let controls = control_room::Controls::read(&output.join("control-room"))?;
    let mut tracker = Tracker {
        run_id: run["run_id"].clone(),
        scope: json!({"corpora":sources.iter().map(|s|&s.name).collect::<Vec<_>>(),
            "size_part":sources.first().map(|s|s.selection.size_part),"order":run["options"]["order"],"limit":run["options"]["limit"]}),
        rows,
        active: BTreeMap::new(),
        preparation: Rate::default(),
        comparison: Rate::default(),
        attempts: AttemptActivity::default(),
        phase: "preparing".into(),
        last: Instant::now(),
        active_seconds: 0.0,
        target: controls.target_groups,
        captures_paused: controls.paused,
        limited: run["options"]["limit"].is_number()
            || run["options"]["prepare_only"] == true
            || run["options"]["diagnose_whitespace_only"] == true,
        finished: false,
    };
    for record in records {
        tracker.observe(record);
    }
    // Resumed history belongs in the totals, never in this invocation's speed.
    tracker.preparation.recent.clear();
    tracker.preparation.finished = 0;
    tracker.comparison.recent.clear();
    tracker.comparison.finished = 0;
    control_room::write(
        &output.join("control-room/progress.json"),
        &tracker.snapshot(provenance::now_ms()),
    )?;
    RUNS.get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(output.to_owned(), tracker);
    Ok(())
}

fn update(output: &Path, change: impl FnOnce(&mut Tracker)) {
    let Some(runs) = RUNS.get() else {
        return;
    };
    let Ok(mut runs) = runs.lock() else {
        return;
    };
    let Some(tracker) = runs.get_mut(output) else {
        return;
    };
    let elapsed = tracker.last.elapsed().as_secs_f64();
    tracker.last = Instant::now();
    tracker.advance(elapsed);
    if let Ok(controls) = control_room::Controls::read(&output.join("control-room")) {
        tracker.target = controls.target_groups;
        tracker.captures_paused = controls.paused;
    }
    change(tracker);
    // Telemetry is derived. A temporarily locked dashboard must not fail a comparison.
    let _ = control_room::write(
        &output.join("control-room/progress.json"),
        &tracker.snapshot(provenance::now_ms()),
    );
}

pub(super) fn tick(output: &Path) {
    update(output, |_| {});
}
pub(super) fn observe(output: &Path, record: &Value) {
    update(output, |tracker| tracker.observe(record));
}
pub(super) fn finish(output: &Path) {
    end(output, "finished");
}
pub(super) fn stop(output: &Path) {
    end(output, "stopped");
}
fn end(output: &Path, phase: &str) {
    update(output, |tracker| {
        tracker.finished = true;
        tracker.phase = phase.into();
    });
}

pub(super) struct ActiveJob {
    output: PathBuf,
    id: String,
    attempt: Option<Attempt>,
}
pub(super) fn start_job(output: &Path, id: &str, mode: JobMode, record: &Value) -> ActiveJob {
    let mut attempt = None;
    update(output, |tracker| {
        tracker.phase = if mode == JobMode::Prepare {
            "preparing"
        } else {
            "comparing"
        }
        .into();
        tracker.active.insert(id.into(), mode);
        if mode == JobMode::Compare {
            attempt = Some(tracker.start_attempt(record));
        }
    });
    ActiveJob {
        output: output.into(),
        id: id.into(),
        attempt,
    }
}
impl ActiveJob {
    pub(super) fn finish(&mut self, record: &Value) {
        if let Some(attempt) = self.attempt.take() {
            update(&self.output, |tracker| {
                tracker.finish_attempt(attempt, record, provenance::now_ms());
            });
        }
    }
}
impl Drop for ActiveJob {
    fn drop(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            update(&self.output, |tracker| {
                tracker.finish_attempt(attempt, &json!({"status":"interrupted",
                    "identity":self.id,"error":"Attempt ended before a final result was recorded."}), provenance::now_ms());
            });
        }
        update(&self.output, |tracker| {
            tracker.active.remove(&self.id);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tracker() -> Tracker {
        Tracker {
            run_id: json!("run"),
            scope: json!({}),
            rows: (0..10).map(|i| (i.to_string(), State::default())).collect(),
            active: BTreeMap::new(),
            preparation: Rate::default(),
            comparison: Rate::default(),
            attempts: AttemptActivity::default(),
            phase: "comparing".into(),
            last: Instant::now(),
            active_seconds: 0.0,
            target: 3,
            captures_paused: false,
            limited: false,
            finished: false,
        }
    }
    fn completed(id: usize) -> Value {
        json!({"identity":id.to_string(),"complete":true,"status":"unchanged"})
    }
    #[test]
    fn resumed_and_repeated_records_never_inflate_speed() {
        let mut t = tracker();
        t.observe(&completed(0));
        t.comparison.finished = 0;
        t.comparison.recent.clear();
        t.advance(60.0);
        t.observe(&completed(0));
        t.observe(&completed(1));
        t.observe(&completed(1));
        let s = t.snapshot(0);
        assert_eq!(s["counts"]["completed"], 2);
        assert_eq!(s["rates"]["new_completions"], 1);
        assert_eq!(s["rates"]["completed_per_hour"], 60.0);
    }
    #[test]
    fn rates_include_inflight_capture_time_but_exclude_drained_pauses() {
        let mut t = tracker();
        t.advance(60.0);
        t.observe(&completed(0));
        t.captures_paused = true;
        t.advance(600.0);
        t.captures_paused = false;
        assert_eq!(t.comparison.seconds, 660.0);
        t.advance(60.0);
        assert_eq!(t.snapshot(0)["rates"]["completed_per_hour"], 5.0);
        t.target = 0;
        t.advance(600.0);
        assert_eq!(t.comparison.seconds, 720.0);
    }
    #[test]
    fn pending_preparation_and_small_samples_do_not_claim_total_eta() {
        let mut t = tracker();
        t.advance(60.0);
        for id in 0..3 {
            t.observe(&completed(id));
        }
        assert_eq!(t.snapshot(0)["eta"]["status"], "preparation_pending");
        for row in t.rows.values_mut() {
            row.prepared = true;
        }
        let s = t.snapshot(1000);
        assert_eq!(s["eta"]["remaining_seconds"], 140.0);
        t.target = 0;
        assert_eq!(t.snapshot(1000)["eta"]["status"], "paused");
        assert!(t.snapshot(1000)["eta"]["finish_unix_ms"].is_null());
    }
    #[test]
    fn skips_and_other_partitions_never_count_as_successful_completions() {
        let mut t = tracker();
        t.advance(60.0);
        t.observe(&json!({"identity":"0","complete":true,"status":"skipped"}));
        t.observe(&completed(100));
        let s = t.snapshot(0);
        assert_eq!(s["counts"]["total"], 10);
        assert_eq!(s["counts"]["remaining"], 9);
        assert_eq!(s["counts"]["skipped"], 1);
        assert_eq!(s["rates"]["new_completions"], 0);
    }

    #[test]
    fn skipped_retries_are_activity_without_new_validated_workbooks() {
        let mut t = tracker();
        let skipped = json!({"identity":"0","name":"Retry.xlsx","complete":true,
            "status":"skipped","error":"Excel opening timed out"});
        t.observe(&skipped);
        assert_eq!(t.snapshot(0)["activity"]["finished_attempts"], 0);
        let mut admitted = skipped.clone();
        admitted["complete"] = json!(false);
        let attempt = t.start_attempt(&admitted);
        t.advance(60.0);
        t.observe(&skipped);
        t.finish_attempt(attempt, &skipped, 60_000);
        let s = t.snapshot(60_000);
        assert_eq!(s["counts"]["skipped"], 1);
        assert_eq!(s["counts"]["remaining"], 9);
        assert_eq!(s["rates"]["new_completions"], 0);
        assert_eq!(s["activity"]["finished_attempts"], 1);
        assert_eq!(s["activity"]["skipped_attempts"], 1);
        assert_eq!(s["activity"]["retried_attempts"], 1);
        assert_eq!(s["activity"]["attempts_per_hour"], 60.0);
        assert_eq!(
            s["activity"]["recent_outcomes"][0]["reason"],
            "Excel opening timed out"
        );
        assert_eq!(s["activity"]["recent_outcomes"][0]["at_unix_ms"], 60_000);
    }

    #[test]
    fn attempt_results_require_explicit_finish_and_keep_bounded_history() {
        let mut t = tracker();
        for id in 0..7 {
            let record = completed(id);
            let attempt = t.start_attempt(&json!({"name":format!("{id}.xlsx")}));
            t.observe(&record);
            assert_eq!(t.attempts.rate.finished, id);
            t.finish_attempt(attempt, &record, id as u64);
        }
        let interrupted = t.start_attempt(&json!({"name":"Interrupted.xlsx"}));
        t.finish_attempt(interrupted, &json!({"status":"interrupted"}), 8);
        let s = t.snapshot(8);
        assert_eq!(s["activity"]["started_attempts"], 8);
        assert_eq!(s["activity"]["finished_attempts"], 8);
        assert_eq!(s["activity"]["compared_attempts"], 7);
        assert_eq!(s["activity"]["interrupted_attempts"], 1);
        assert_eq!(s["activity"]["retried_attempts"], 0);
        assert_eq!(
            s["activity"]["recent_outcomes"].as_array().unwrap().len(),
            5
        );
        assert_eq!(s["activity"]["recent_outcomes"][0]["status"], "interrupted");
        assert_eq!(s["counts"]["completed"], 7);
    }
    #[test]
    fn a_stall_ages_old_completions_out_of_the_rolling_window() {
        let mut t = tracker();
        t.advance(60.0);
        for id in 0..3 {
            t.observe(&completed(id));
        }
        t.advance(901.0);
        assert_eq!(t.snapshot(0)["rates"]["completed_per_hour"], 0.0);
        assert!(t.snapshot(0)["eta"]["remaining_seconds"].is_null());
    }

    #[test]
    fn stopped_run_is_neither_paused_nor_forecast_as_still_working() {
        let mut t = tracker();
        t.target = 0;
        t.finished = true;
        t.phase = "stopped".into();
        let snapshot = t.snapshot(0);
        assert_eq!(snapshot["phase"], "stopped");
        assert_eq!(snapshot["paused"], false);
        assert_eq!(snapshot["eta"]["status"], "unavailable");
    }
}
