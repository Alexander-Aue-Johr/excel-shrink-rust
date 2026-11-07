//! Aggregate timings by operation and sheet instead of retaining every request.
use super::{Snapshot, StepStatus};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Default, Serialize)]
pub struct PhaseTiming {
    pub elapsed_ms: u64,
    pub started_at: Option<i64>,
    pub calls: u64,
}

impl PhaseTiming {
    /// Repeated running snapshots keep the original start of the same interval.
    pub fn transition(&mut self, status: StepStatus, now: i64) {
        if status == StepStatus::Running {
            if self.started_at.is_none() {
                self.started_at = Some(now);
                self.calls += 1;
            }
        } else if let Some(started) = self.started_at.take() {
            self.elapsed_ms += now.saturating_sub(started).max(0) as u64;
        }
    }
}

#[derive(Serialize)]
pub struct OperationTiming {
    pub lane: Option<String>,
    pub sheet: Option<String>,
    pub operation: String,
    pub calls: u64,
    pub failed: u64,
    pub elapsed_ms: u64,
    pub geometry_ms: f64,
    pub render_ms: f64,
    pub upload_ms: f64,
}

impl Snapshot {
    /// Retain per-lane response latency when available; batch wall time stays unassigned.
    pub fn record_operation(&mut self, command: &Value, response: &Value, elapsed_ms: u64) {
        let jobs = command["jobs"].as_array();
        let results = response["result"]["images"].as_array();
        if let Some((jobs, results)) = jobs.zip(results).filter(|(jobs, results)| {
            jobs.len() == results.len()
                && results
                    .iter()
                    .all(|result| result["operation_timing"]["elapsed_ms"].as_u64().is_some())
        }) {
            for (job, result) in jobs.iter().zip(results) {
                self.add_operation(
                    job,
                    &result["operation_timing"],
                    elapsed_ms,
                    response["ok"] != true,
                );
            }
        } else {
            self.add_operation(
                command,
                &response["result"]["operation_timing"],
                elapsed_ms,
                response["ok"] != true,
            );
        }
    }

    fn add_operation(&mut self, command: &Value, timing: &Value, fallback_ms: u64, failed: bool) {
        let lane = command["lane"].as_str().map(str::to_owned);
        let sheet = command["sheet"].as_str().map(str::to_owned);
        let operation = command["kind"].as_str().unwrap_or("unknown").to_owned();
        let index = self
            .operation_timings
            .iter()
            .position(|item| {
                item.lane == lane && item.sheet == sheet && item.operation == operation
            })
            .unwrap_or_else(|| {
                self.operation_timings.push(OperationTiming {
                    lane,
                    sheet,
                    operation,
                    calls: 0,
                    failed: 0,
                    elapsed_ms: 0,
                    geometry_ms: 0.0,
                    render_ms: 0.0,
                    upload_ms: 0.0,
                });
                self.operation_timings.len() - 1
            });
        let item = &mut self.operation_timings[index];
        item.calls += 1;
        item.failed += u64::from(failed);
        item.elapsed_ms += timing["elapsed_ms"].as_u64().unwrap_or(fallback_ms);
        for (key, total) in [
            ("geometry_ms", &mut item.geometry_ms),
            ("render_ms", &mut item.render_ms),
            ("binary_upload_ms", &mut item.upload_ms),
        ] {
            *total += timing["client"][key]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_running_updates_do_not_restart_phase_clocks() {
        let mut timing = PhaseTiming::default();
        timing.transition(StepStatus::Running, 100);
        timing.transition(StepStatus::Running, 130);
        timing.transition(StepStatus::Done, 180);
        timing.transition(StepStatus::Running, 200);
        timing.transition(StepStatus::Failed, 220);
        assert_eq!(timing.elapsed_ms, 100);
        assert_eq!(timing.calls, 2);
        assert_eq!(timing.started_at, None);
    }
}
