use std::collections::{HashSet, VecDeque};

use serde_json::{json, Value};

use super::now_iso;

const HISTORY_LIMIT: usize = 24;

/// Protocol observations, not guesses about Excel's internal activity. Heartbeat
/// samples and idle polls never count as work or extend the event history.
pub(super) struct WorkerTelemetry {
    state: String,
    wait_reason: Option<String>,
    state_since: String,
    last_progress: Option<String>,
    last_event: Value,
    last_success: Value,
    active_job: Value,
    events: VecDeque<Value>,
    sequence: u64,
    uploaded_images: u64,
    completed_commands: u64,
    pane_resources: Value,
}

impl WorkerTelemetry {
    pub fn new() -> Self {
        Self {
            state: "starting".into(),
            wait_reason: None,
            state_since: now_iso(),
            last_progress: None,
            last_event: Value::Null,
            last_success: Value::Null,
            active_job: Value::Null,
            events: VecDeque::new(),
            sequence: 0,
            uploaded_images: 0,
            completed_commands: 0,
            pane_resources: Value::Null,
        }
    }

    pub fn sample_pane(&mut self, source: &Value) {
        let mut sample = json!({"sampled_at": now_iso()});
        for key in [
            "retained_capture_plans",
            "retained_tiles",
            "geometry_cache_entries",
            "geometry_rows",
            "pending_png_characters",
            "active_batch_pixels",
            "active_batch_tiles",
            "js_heap_used_bytes",
            "js_heap_limit_bytes",
            "script_evaluations",
        ] {
            if source[key]
                .as_f64()
                .is_some_and(|value| value.is_finite() && value >= 0.0)
            {
                sample[key] = source[key].clone();
            }
        }
        self.pane_resources = sample;
    }

    pub fn transition(&mut self, next: &str, waiting_for: Option<&str>, message: &str) {
        if self.state == next && self.wait_reason.as_deref() == waiting_for {
            return;
        }
        self.state = next.to_owned();
        self.wait_reason = waiting_for.map(str::to_owned);
        self.state_since = now_iso();
        self.record("state_changed", message, Value::Null);
    }

    pub fn configured(&mut self) {
        self.transition(
            "verifying_workbook",
            Some("excel"),
            "Add-in configured; verifying the workbook through Office.js.",
        );
        self.progress(
            "addin_configured",
            "Add-in configuration completed.",
            Value::Null,
        );
    }

    pub fn ready(&mut self) {
        self.transition(
            "waiting_for_startup_sample",
            Some("launcher"),
            "Workbook verified; waiting for the launcher's memory sample.",
        );
        self.progress(
            "addin_ready",
            "Office.js synchronized and verified the staged workbook identity.",
            Value::Null,
        );
    }

    pub fn configuration_attempt(
        &mut self,
        document: &str,
        run: &str,
        request: &str,
        outcome: &str,
    ) {
        self.record("configuration_request", &format!("Taskpane configuration request: {outcome}."),
            json!({"document_id": document, "run_id": run, "request_id": request, "outcome": outcome}));
    }

    pub fn started(&mut self, command: &Value) {
        self.active_job = describe(command);
        let kind = command["kind"].as_str().unwrap_or_default();
        let state = match kind {
            "plan" => "planning_sheet",
            "images" => "providing_images",
            "release" => "releasing",
            _ => "capturing",
        };
        self.transition(state, None, &format!("Office.js received {kind} command."));
        self.record(
            "job_dispatched",
            "Command delivered to the add-in.",
            self.active_job.clone(),
        );
    }

    pub fn image_received(&mut self, tile: &Value) {
        self.uploaded_images += 1;
        let mut details = pick(tile, &["sheet_index", "sheet_name", "tile_index", "range"]);
        details["command_id"] = self.active_job["id"].clone();
        self.progress(
            "image_received",
            "Capture image received and validated.",
            details,
        );
    }

    pub fn finished(&mut self, ok: bool, error: &Value) {
        if ok {
            self.completed_commands += 1;
            self.progress(
                "job_completed",
                "Office.js command completed successfully.",
                self.active_job.clone(),
            );
            self.transition(
                "waiting_for_job",
                Some("coordinator"),
                "Waiting for the next coordinator command.",
            );
        } else {
            let state = if error["code"] == "InvalidOperationInCellEditMode" {
                "blocked_edit_mode"
            } else {
                "client_error"
            };
            self.transition(
                state,
                None,
                error["message"]
                    .as_str()
                    .unwrap_or("Office.js reported a failed command."),
            );
            self.record(
                "job_failed",
                error["message"].as_str().unwrap_or("Command failed."),
                self.active_job.clone(),
            );
        }
        self.active_job = Value::Null;
    }

    pub fn restarted(&mut self) {
        self.transition("client_error", None, "A different taskpane document appeared after a command was dispatched; this is not proof of a WebView crash.");
    }

    fn progress(&mut self, kind: &str, message: &str, details: Value) {
        self.last_progress = Some(now_iso());
        self.last_success = self.record(kind, message, details);
    }

    fn record(&mut self, kind: &str, message: &str, details: Value) -> Value {
        self.sequence += 1;
        let mut entry =
            json!({"sequence": self.sequence, "at": now_iso(), "kind": kind, "message": message});
        if kind == "state_changed" {
            entry["state"] = json!(self.state);
            entry["wait_reason"] = json!(self.wait_reason);
        }
        if !details.is_null() {
            entry["details"] = details;
        }
        self.events.push_back(entry.clone());
        while self.events.len() > HISTORY_LIMIT {
            self.events.pop_front();
        }
        self.last_event = entry.clone();
        entry
    }

    pub fn snapshot(&self) -> Value {
        json!({
            "state": self.state,
            "state_since": self.state_since,
            "wait_reason": self.wait_reason,
            "last_progress_at": self.last_progress,
            "last_event": self.last_event,
            "last_success": self.last_success,
            "active_job": self.active_job,
            "events": self.events,
            "uploaded_images": self.uploaded_images,
            "completed_commands": self.completed_commands,
            "pane_resources": self.pane_resources,
        })
    }
}

fn describe(command: &Value) -> Value {
    let mut result = pick(command, &["id", "kind", "sheet_index", "sheet_name"]);
    let tiles: Vec<_> = command["tiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
        .collect();
    let sheets: Vec<_> = command["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
        .collect();
    result["tile_count"] = json!(tiles.len());
    result["sheet_count"] = json!(if tiles.is_empty() {
        sheets.len()
    } else {
        tiles
            .iter()
            .map(|tile| tile["sheet_index"].to_string())
            .collect::<HashSet<_>>()
            .len()
    });
    let locations = if tiles.is_empty() { sheets } else { tiles };
    result["locations"] = json!(locations
        .into_iter()
        .take(8)
        .map(|item| {
            pick(
                item,
                &[
                    "sheet_index",
                    "sheet_name",
                    "tile_index",
                    "range",
                    "name",
                    "index",
                ],
            )
        })
        .collect::<Vec<_>>());
    result
}

fn pick(source: &Value, names: &[&str]) -> Value {
    let mut result = json!({});
    for name in names {
        if let Some(value) = source.get(*name).filter(|value| !value.is_null()) {
            result[*name] = value.clone();
        }
    }
    result
}
