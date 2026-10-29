//! Assignment progress derived from unique workbook records, never profile counts.
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const WINDOW: f64 = 900.0;
static RUNS: OnceLock<Mutex<BTreeMap<PathBuf, Tracker>>> = OnceLock::new();

#[derive(Clone, Copy, Default)]
struct State {
    completed: bool,
    full_checked: bool,
    screen_completed: bool,
    primary_checked: bool,
    primary_failed: bool,
    skipped: bool,
    deferred: bool,
    prepared: bool,
    preparation_terminal: bool,
    visual_eligible: bool,
    diagnosis_failed: bool,
}

impl State {
    fn from_record(record: &Value) -> Self {
        let completed = record["complete"] == true
            && matches!(record["status"].as_str(), Some("unchanged" | "differences"))
            && !whitespace::needed(record)
            && (!strategy::is_priority(record) || strategy::full_complete(record));
        let full_checked = strategy::full_complete(record);
        let screen_completed = record["complete"] == true
            && record["comparison_hash_profile"].is_string()
            && record["comparison_hash_profile"] == record["last_hash_screen_profile"]
            && matches!(
                record["status"].as_str(),
                Some(
                    "screened-equal"
                        | "unchanged"
                        | "differences"
                        | "crc-identical"
                        | "workbook-only"
                )
            );
        let primary_checked =
            full_checked || record["priority_validation"]["primary"]["status"] == "complete";
        Self {
            completed,
            full_checked,
            screen_completed,
            primary_checked,
            primary_failed: !primary_checked
                && record["priority_validation"]["primary"]["status"] == "failed",
            skipped: record["complete"] == true && record["status"] == "skipped",
            deferred: record["complete"] == true && record["status"] == "paused-for-later",
            prepared: record["prepared"] == true || completed,
            preparation_terminal: record["preparation_disposition"].is_string(),
            visual_eligible: record["preparation_disposition"] == "needs_visual",
            diagnosis_failed: record["whitespace_diagnosis"]["status"] == "failed",
        }
    }

    fn full_completed(&self, comparison_pass: &str) -> bool {
        if comparison_pass == strategy::HASH_SCREEN {
            return self.screen_completed;
        }
        if matches!(comparison_pass, strategy::PRIMARY | strategy::SECONDARY) {
            self.full_checked
        } else {
            self.completed
        }
    }

    fn completed_for_pass(&self, comparison_pass: &str) -> bool {
        if comparison_pass == strategy::PRIMARY {
            self.primary_checked
        } else {
            self.full_completed(comparison_pass)
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
    names: BTreeMap<String, String>,
    active: BTreeMap<String, JobMode>,
    preparation: Rate,
    comparison: Rate,
    attempts: AttemptActivity,
    phase: String,
    comparison_pass: String,
    last: Instant,
    active_seconds: f64,
    target: usize,
    captures_paused: bool,
    limited: bool,
    finished: bool,
    visual_scope_frozen: bool,
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
                Some("skipped" | "unchanged" | "differences" | "screened-equal")
            ) || record["last_execution"]["mode"] == "Compare",
            workbook: record["name"].as_str().unwrap_or("Unknown workbook").into(),
        }
    }

    fn finish_attempt(&mut self, attempt: Attempt, record: &Value, now_ms: u64) {
        let status = match record["status"].as_str() {
            Some("unchanged" | "differences" | "screened-equal") if record["complete"] == true => {
                self.attempts.compared += 1;
                record["status"].as_str().unwrap()
            }
            Some("screened" | "screening-differences")
                if record["priority_validation"]["primary"]["status"] == "complete" =>
            {
                self.attempts.compared += 1;
                record["status"].as_str().unwrap()
            }
            Some("screening-failed" | "validation-incomplete") => {
                self.attempts.skipped += 1;
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
        if state.completed_for_pass(&self.comparison_pass) {
            self.comparison.record(id);
        }
        if state.preparation_terminal || state.prepared {
            self.preparation.record(id);
        }
        self.rows.insert(id.to_owned(), state);
    }
    fn set_comparison_pass(&mut self, comparison_pass: &str) {
        if self.comparison_pass == comparison_pass {
            return;
        }
        self.comparison_pass = comparison_pass.to_owned();
        self.comparison = Rate::default();
        for (id, state) in &self.rows {
            if state.completed_for_pass(comparison_pass) {
                self.comparison.seen.insert(id.clone());
            }
        }
    }

    fn paused(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "waiting for disk" | "awaiting_deferred_manual"
        ) || (!self.finished
            && self.phase != "preparing"
            && ((self.target == 0 && self.active.is_empty()) || self.captures_paused))
    }
    fn advance(&mut self, seconds: f64) {
        // Capture pause only prevents new jobs; already dispatched jobs may
        // still finish. Keep their wall time in the denominator. Only a fully
        // drained workbook pause proves that no work is being performed.
        if self.finished
            || self.phase == "awaiting_deferred_manual"
            || (self.phase != "preparing" && self.target == 0 && self.active.is_empty())
        {
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
        let preparation_total = self.rows.len();
        let preparation_terminal = self
            .rows
            .values()
            .filter(|row| row.preparation_terminal)
            .count();
        let preparation_remaining = preparation_total - preparation_terminal;
        let visual_total = self.rows.values().filter(|row| row.visual_eligible).count();
        let visual_completed = self
            .rows
            .values()
            .filter(|row| row.visual_eligible && row.full_completed(&self.comparison_pass))
            .count();
        let visual_skipped = self
            .rows
            .values()
            .filter(|row| row.visual_eligible && row.skipped)
            .count();
        let visual_deferred = self
            .rows
            .values()
            .filter(|row| row.visual_eligible && row.deferred)
            .count();
        let full_remaining = if matches!(
            self.comparison_pass.as_str(),
            strategy::PRIMARY | strategy::SECONDARY
        ) {
            // A failed or deferred attempt is still missing independent full
            // validation, even though ordinary runs treat it as terminal work.
            visual_total - visual_completed
        } else {
            visual_total - visual_completed - visual_skipped - visual_deferred
        };
        let primary_completed = self
            .rows
            .values()
            .filter(|row| row.visual_eligible && row.primary_checked)
            .count();
        let primary_failed = self
            .rows
            .values()
            .filter(|row| row.visual_eligible && row.primary_failed)
            .count();
        let primary_remaining = visual_total.saturating_sub(primary_completed + primary_failed);
        let visual_remaining = if self.comparison_pass == strategy::PRIMARY {
            primary_remaining
        } else {
            full_remaining
        };
        let visual_excluded = self
            .rows
            .values()
            .filter(|row| row.preparation_terminal && !row.visual_eligible)
            .count();
        let preparation_skipped = self
            .rows
            .values()
            .filter(|row| row.preparation_terminal && row.skipped)
            .count();
        let (total, completed, skipped, remaining) = if self.visual_scope_frozen {
            if self.comparison_pass == strategy::PRIMARY {
                (
                    visual_total,
                    primary_completed,
                    primary_failed,
                    visual_remaining,
                )
            } else {
                (
                    visual_total,
                    visual_completed,
                    visual_skipped,
                    visual_remaining,
                )
            }
        } else {
            (
                preparation_total,
                preparation_terminal - preparation_skipped,
                preparation_skipped,
                preparation_remaining,
            )
        };
        let prepared = self.rows.values().filter(|row| row.prepared).count();
        let active_preparation: Vec<_> = self
            .active
            .iter()
            .filter(|(_, mode)| **mode == JobMode::Prepare)
            .map(|(id, _)| json!({"id":id,"workbook":self.names.get(id)}))
            .collect();
        let phase = if self.finished {
            &self.phase
        } else if self.target == 0
            && !matches!(
                self.phase.as_str(),
                "preparing" | "awaiting_deferred_manual"
            )
        {
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
        let comparison_eta = self.comparison.estimate(visual_remaining);
        let (eta_status, eta) = if self.phase == "waiting for disk" {
            ("waiting_for_disk", None)
        } else if self.phase == "awaiting_deferred_manual" {
            ("awaiting_manual", None)
        } else if remaining == 0 {
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
        json!({"version":2,"run_id":self.run_id,"scope":self.scope,"phase":phase,"paused":paused,
            "visual_scope_frozen":self.visual_scope_frozen,"active_preparation":active_preparation,
            "updated_unix_ms":now_ms,
            "counts":{"total":total,"completed":completed,"skipped":skipped,"remaining":remaining,
                "in_progress":self.active.len(),"prepared":prepared,
                "preparation_total":preparation_total,"preparation_terminal":preparation_terminal,
                "preparation_remaining":preparation_remaining,
                "visual_total":self.visual_scope_frozen.then_some(visual_total),
                "visual_completed":self.visual_scope_frozen.then_some(visual_completed),
                "visual_deferred":self.visual_scope_frozen.then_some(visual_deferred),
                "visual_remaining":self.visual_scope_frozen.then_some(visual_remaining),
                "visual_excluded":self.visual_scope_frozen.then_some(visual_excluded)},
            "priority_validation":{"active_pass":self.comparison_pass,"primary_completed":primary_completed,
                "primary_failed":primary_failed,"primary_remaining":primary_remaining,
                "full_completed":visual_completed,"full_remaining":full_remaining},
            "diagnosis_failed":self.rows.values().filter(|row| row.diagnosis_failed).count(),
            "rates":{"window_seconds":WINDOW,"completed_per_hour":rate,"completed_per_minute":rate.map(|rate|rate/60.0),
                "average_completed_per_hour":if self.active_seconds>0.0 {Some(self.comparison.finished as f64*3600.0/self.active_seconds)} else {None},
                "preparation_per_hour":self.preparation.per_hour(),
                "preparation_per_minute":self.preparation.per_hour().map(|rate|rate/60.0),
                "samples":self.comparison.recent.len(),
                "active_seconds":self.active_seconds,"comparison_active_seconds":self.comparison.seconds,
                "preparation_active_seconds":self.preparation.seconds,"new_completions":self.comparison.finished},
            "activity":{"source":"runner","started_attempts":self.attempts.started,"finished_attempts":self.attempts.rate.finished,
                "compared_attempts":self.attempts.compared,"skipped_attempts":self.attempts.skipped,
                "interrupted_attempts":self.attempts.interrupted,"retried_attempts":self.attempts.retried,
                "attempts_per_hour":self.attempts.rate.per_hour(),"recent_outcomes":self.attempts.outcomes},
            "eta":{"status":eta_status,"remaining_seconds":eta,
                "preparation_remaining_seconds":preparation_eta,
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
    let names: BTreeMap<_, _> = sources
        .iter()
        .flat_map(|source| {
            source
                .members
                .iter()
                .filter(|(index, _)| source.selection.contains(*index))
                .map(|(index, name)| (identity(source, *index, name), name.clone()))
        })
        .collect();
    let rows = names
        .keys()
        .cloned()
        .map(|id| (id, State::default()))
        .collect();
    let controls = control_room::Controls::read(&output.join("control-room"))?;
    let mut tracker = Tracker {
        run_id: run["run_id"].clone(),
        scope: json!({"corpora":sources.iter().map(|s|&s.name).collect::<Vec<_>>(),
            "size_part":sources.first().map(|s|s.selection.size_part),"order":run["options"]["order"],"limit":run["options"]["limit"]}),
        rows,
        names,
        active: BTreeMap::new(),
        preparation: Rate::default(),
        comparison: Rate::default(),
        attempts: AttemptActivity::default(),
        phase: "preparing".into(),
        comparison_pass: if run["options"]["preparation_hash_profile"].is_string() {
            strategy::HASH_SCREEN.into()
        } else if run["options"]["aggressive_first"] == true {
            strategy::PRIMARY.into()
        } else {
            "full".into()
        },
        last: Instant::now(),
        active_seconds: 0.0,
        target: controls.target_groups,
        captures_paused: controls.paused,
        limited: run["options"]["limit"].is_number()
            || run["options"]["prepare_only"] == true
            || run["options"]["diagnose_whitespace_only"] == true,
        finished: false,
        visual_scope_frozen: false,
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
pub(super) fn set_selection(output: &Path, selected: &HashSet<String>) {
    update(output, |tracker| {
        tracker.rows.retain(|id, _| selected.contains(id));
        tracker.names.retain(|id, _| selected.contains(id));
    });
}
pub(super) fn set_comparison_pass(output: &Path, comparison_pass: &str) {
    update(output, |tracker| {
        tracker.set_comparison_pass(comparison_pass)
    });
}

pub(super) fn freeze_visual_scope(output: &Path) {
    update(output, |tracker| tracker.visual_scope_frozen = true);
}
pub(super) fn selection_scope(output: &Path) -> Option<(Value, HashSet<String>, bool)> {
    let runs = RUNS.get()?.lock().ok()?;
    let tracker = runs.get(output)?;
    Some((
        tracker.run_id.clone(),
        tracker.rows.keys().cloned().collect(),
        tracker.visual_scope_frozen,
    ))
}
pub(super) fn finish(output: &Path) {
    end(output, "finished");
}
pub(super) fn stop(output: &Path) {
    end(output, "stopped");
}
pub(super) fn wait_for_disk(output: &Path) {
    end(output, "waiting for disk");
}
pub(super) fn await_deferred(output: &Path) {
    update(output, |tracker| {
        tracker.phase = "awaiting_deferred_manual".into()
    });
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
            names: BTreeMap::new(),
            active: BTreeMap::new(),
            preparation: Rate::default(),
            comparison: Rate::default(),
            attempts: AttemptActivity::default(),
            phase: "comparing".into(),
            comparison_pass: "full".into(),
            last: Instant::now(),
            active_seconds: 0.0,
            target: 3,
            captures_paused: false,
            limited: false,
            finished: false,
            visual_scope_frozen: false,
        }
    }
    fn completed(id: usize) -> Value {
        json!({"identity":id.to_string(),"complete":true,"status":"unchanged",
            "preparation_disposition":"needs_visual"})
    }
    fn visual_tracker() -> Tracker {
        let mut tracker = tracker();
        for row in tracker.rows.values_mut() {
            row.preparation_terminal = true;
            row.visual_eligible = true;
        }
        tracker.visual_scope_frozen = true;
        tracker
    }
    #[test]
    fn resumed_and_repeated_records_never_inflate_speed() {
        let mut t = visual_tracker();
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
    fn zero_visual_groups_does_not_pause_preparation() {
        let mut t = tracker();
        t.phase = "preparing".into();
        t.target = 0;
        assert!(!t.paused());
        assert_eq!(t.snapshot(0)["phase"], "preparing");
        t.advance(60.0);
        assert_eq!(t.preparation.seconds, 60.0);

        t.phase = "waiting for disk".into();
        t.finished = true;
        assert!(t.paused());
        assert_eq!(t.snapshot(0)["eta"]["status"], "waiting_for_disk");
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
            row.preparation_terminal = true;
            row.visual_eligible = true;
        }
        t.visual_scope_frozen = true;
        let s = t.snapshot(1000);
        assert_eq!(s["eta"]["remaining_seconds"], 140.0);
        t.target = 0;
        assert_eq!(t.snapshot(1000)["eta"]["status"], "paused");
        assert!(t.snapshot(1000)["eta"]["finish_unix_ms"].is_null());
    }
    #[test]
    fn skips_and_other_partitions_never_count_as_successful_completions() {
        let mut t = visual_tracker();
        t.advance(60.0);
        t.observe(&json!({"identity":"0","complete":true,"status":"skipped",
            "preparation_disposition":"needs_visual"}));
        t.observe(&completed(100));
        let s = t.snapshot(0);
        assert_eq!(s["counts"]["total"], 10);
        assert_eq!(s["counts"]["remaining"], 9);
        assert_eq!(s["counts"]["skipped"], 1);
        assert_eq!(s["rates"]["new_completions"], 0);
    }

    #[test]
    fn visual_denominator_is_frozen_after_preparation_and_excludes_nonvisual_files() {
        let mut t = tracker();
        for id in 0..3 {
            t.observe(&json!({"identity":id.to_string(),"prepared":true,
                "preparation_disposition":"needs_visual","status":"prepared"}));
        }
        for id in 3..5 {
            t.observe(
                &json!({"identity":id.to_string(),"prepared":true,"complete":true,
                "preparation_disposition":"crc_unchanged","status":"crc-identical"}),
            );
        }
        t.observe(&json!({"identity":"5","complete":true,
            "preparation_disposition":"unsupported_namespace","status":"skipped"}));
        let preparing = t.snapshot(0);
        assert_eq!(preparing["counts"]["preparation_total"], 10);
        assert_eq!(preparing["counts"]["preparation_terminal"], 6);
        assert!(preparing["counts"]["visual_total"].is_null());

        t.visual_scope_frozen = true;
        let comparing = t.snapshot(0);
        assert_eq!(comparing["counts"]["visual_total"], 3);
        assert_eq!(comparing["counts"]["visual_excluded"], 3);
        assert_eq!(comparing["counts"]["remaining"], 3);
        t.observe(&json!({"identity":"0","prepared":true,"complete":true,
            "preparation_disposition":"needs_visual","status":"unchanged"}));
        t.observe(&json!({"identity":"1","prepared":true,"complete":true,
            "preparation_disposition":"needs_visual","status":"skipped"}));
        t.observe(&json!({"identity":"2","prepared":true,"complete":true,
            "preparation_disposition":"needs_visual","status":"paused-for-later"}));
        let after = t.snapshot(0);
        assert_eq!(after["counts"]["visual_completed"], 1);
        assert_eq!(after["counts"]["skipped"], 1);
        assert_eq!(after["counts"]["visual_deferred"], 1);
        assert_eq!(after["counts"]["visual_remaining"], 0);
    }

    #[test]
    fn skipped_retries_are_activity_without_new_validated_workbooks() {
        let mut t = visual_tracker();
        let skipped = json!({"identity":"0","name":"Retry.xlsx","complete":true,
            "status":"skipped","error":"Excel opening timed out",
            "preparation_disposition":"needs_visual"});
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
        let mut t = visual_tracker();
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
    #[test]
    fn priority_counts_distinguish_screening_from_full_completion() {
        let mut tracker = visual_tracker();
        tracker.comparison_pass = strategy::PRIMARY.into();
        let mut record = json!({"identity":"0","complete":false,"status":"screened",
            "preparation_disposition":"needs_visual","comparison_pass":strategy::PRIMARY,
            "priority_validation":{"primary":{"status":"complete"}},
            "results":[{"profile":strategy::AGGRESSIVE,"checks_complete":true}]});
        tracker.observe(&record);
        let snapshot = tracker.snapshot(0);
        assert_eq!(snapshot["counts"]["completed"], 1);
        assert_eq!(snapshot["priority_validation"]["full_completed"], 0);
        tracker.comparison_pass = strategy::SECONDARY.into();
        record["comparison_pass"] = json!(strategy::SECONDARY);
        record["complete"] = json!(true);
        record["status"] = json!("differences");
        tracker.observe(&record);
        assert_eq!(tracker.snapshot(0)["counts"]["completed"], 0);
    }
    #[test]
    fn recovered_full_checks_are_not_also_failed_screening_work() {
        let mut tracker = visual_tracker();
        tracker.comparison_pass = strategy::PRIMARY.into();
        let mut recovered = json!({"identity":"0","complete":true,"status":"unchanged",
            "preparation_disposition":"needs_visual","comparison_pass":strategy::SECONDARY,
            "priority_validation":{"primary":{"status":"failed"}},"results":[]});
        for profile in prepared::PROFILES {
            recovered["results"].as_array_mut().unwrap().push(
                json!({"profile":profile,"checks_complete":true,"image_comparison":"complete"}),
            );
        }
        tracker.observe(&recovered);
        tracker.observe(
            &json!({"identity":"1","preparation_disposition":"needs_visual",
            "priority_validation":{"primary":{"status":"failed"}}}),
        );
        let snapshot = tracker.snapshot(0);
        assert_eq!(snapshot["priority_validation"]["primary_completed"], 1);
        assert_eq!(snapshot["priority_validation"]["primary_failed"], 1);
        assert_eq!(snapshot["priority_validation"]["primary_remaining"], 8);
    }

    #[test]
    fn experimental_progress_requires_full_checks_for_legacy_results() {
        // Ordinary historical runs can be complete after a partial capture.
        // Their old completion flag retains its meaning in the normal workflow.
        let legacy = json!({"identity":"0","complete":true,"status":"differences",
        "whitespace_diagnosis":{"version":whitespace::VERSION,"status":"not_applicable"},
        "preparation_disposition":"needs_visual","results":[
            {"profile":strategy::AGGRESSIVE,"checks_complete":true},
            {"profile":"default","checks_complete":false},
            {"profile":"maximum-cleaning","checks_complete":false}
        ]});
        let mut normal = visual_tracker();
        normal.observe(&legacy);
        assert_eq!(normal.snapshot(0)["counts"]["completed"], 1);
        assert_eq!(normal.comparison.finished, 1);

        let mut experimental = visual_tracker();
        experimental.set_comparison_pass(strategy::PRIMARY);
        experimental.observe(&legacy);
        let primary = experimental.snapshot(0);
        assert_eq!(primary["priority_validation"]["primary_completed"], 0);
        assert_eq!(primary["priority_validation"]["full_completed"], 0);
        assert_eq!(primary["priority_validation"]["full_remaining"], 10);
        assert_eq!(experimental.comparison.finished, 0);
        experimental.set_comparison_pass(strategy::SECONDARY);
        experimental.observe(&legacy);
        let secondary = experimental.snapshot(0);
        assert_eq!(secondary["counts"]["completed"], 0);
        assert_eq!(secondary["counts"]["visual_completed"], 0);
        assert_eq!(secondary["counts"]["remaining"], 10);
        assert_eq!(secondary["rates"]["new_completions"], 0);
        assert!(!experimental.comparison.seen.contains("0"));
    }

    #[test]
    fn experimental_full_coverage_keeps_failed_and_deferred_records_pending() {
        let mut tracker = visual_tracker();
        let mut checked = completed(0);
        checked["results"] = json!(prepared::PROFILES
            .iter()
            .map(
                |name| json!({"profile":name,"checks_complete":true,"image_comparison":"complete"})
            )
            .collect::<Vec<_>>());
        tracker.observe(&checked);
        tracker.observe(&json!({"identity":"1","complete":true,"status":"skipped",
            "preparation_disposition":"needs_visual"}));
        tracker.observe(
            &json!({"identity":"2","complete":true,"status":"paused-for-later",
            "preparation_disposition":"needs_visual"}),
        );
        tracker.observe(&json!({"identity":"3","complete":false,"status":"screening-failed",
            "preparation_disposition":"needs_visual","priority_validation":{"primary":{"status":"failed"}}}));
        // Ordinary runs retain their historical terminal-work accounting.
        assert_eq!(tracker.snapshot(0)["counts"]["remaining"], 7);
        tracker.set_comparison_pass(strategy::PRIMARY);
        let primary = tracker.snapshot(0);
        assert_eq!(primary["priority_validation"]["full_completed"], 1);
        assert_eq!(primary["priority_validation"]["full_remaining"], 9);
        tracker.set_comparison_pass(strategy::SECONDARY);
        let secondary = tracker.snapshot(0);
        assert_eq!(secondary["counts"]["remaining"], 9);
        assert_eq!(secondary["counts"]["visual_remaining"], 9);
        assert_eq!(secondary["priority_validation"]["full_remaining"], 9);
        assert_eq!(secondary["counts"]["skipped"], 1);
        assert_eq!(secondary["counts"]["visual_deferred"], 1);
    }

    #[test]
    fn experimental_secondary_rates_seed_only_fully_checked_legacy_results() {
        let mut tracker = visual_tracker();
        tracker.set_comparison_pass(strategy::PRIMARY);
        let mut legacy = completed(0);
        legacy["results"] = json!([{"profile":strategy::AGGRESSIVE,"checks_complete":true}]);
        tracker.observe(&legacy);
        let mut checked = completed(1);
        checked["results"] = json!(prepared::PROFILES
            .iter()
            .map(
                |name| json!({"profile":name,"checks_complete":true,"image_comparison":"complete"})
            )
            .collect::<Vec<_>>());
        tracker.observe(&checked);
        tracker.set_comparison_pass(strategy::SECONDARY);
        assert_eq!(tracker.comparison.seen, HashSet::from(["1".to_owned()]));
        tracker.observe(&checked);
        tracker.observe(&legacy);
        assert_eq!(tracker.comparison.finished, 0);
        assert_eq!(
            tracker.snapshot(0)["priority_validation"]["full_completed"],
            1
        );
        legacy["results"] = checked["results"].clone();
        tracker.advance(60.0);
        tracker.observe(&legacy);
        tracker.observe(&legacy);
        let snapshot = tracker.snapshot(0);
        assert_eq!(snapshot["priority_validation"]["full_completed"], 2);
        assert_eq!(snapshot["rates"]["new_completions"], 1);
        assert_eq!(snapshot["rates"]["completed_per_hour"], 60.0);
    }

    #[test]
    fn screening_success_is_a_finished_attempt_without_full_completion() {
        let mut tracker = visual_tracker();
        tracker.comparison_pass = strategy::PRIMARY.into();
        let record = json!({"identity":"0","name":"Screen.xlsx","complete":false,"status":"screened",
            "preparation_disposition":"needs_visual","comparison_pass":strategy::PRIMARY,
            "priority_validation":{"primary":{"status":"complete"}},
            "results":[{"profile":strategy::AGGRESSIVE,"checks_complete":true}]});
        let attempt = tracker.start_attempt(&record);
        tracker.observe(&record);
        tracker.finish_attempt(attempt, &record, 1000);
        let snapshot = tracker.snapshot(1000);
        assert_eq!(snapshot["activity"]["compared_attempts"], 1);
        assert_eq!(snapshot["activity"]["interrupted_attempts"], 0);
        assert_eq!(snapshot["priority_validation"]["full_completed"], 0);
    }
}
