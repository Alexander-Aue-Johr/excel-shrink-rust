//! Bounded live projection of scheduler events, independent of visual outcomes.

use crate::validation_process::{ProcessOutcome, ProcessStatus};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Serialize)]
pub(crate) struct ProcessActivity {
    pub invocation: usize,
    pub name: String,
    pub input_count: usize,
    pub status: ProcessStatus,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct ProcessingProgress {
    pub stage: String,
    pub step: usize,
    pub total: usize,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub running: BTreeMap<usize, ProcessActivity>,
    pub recent: VecDeque<ProcessActivity>,
}

impl ProcessingProgress {
    pub fn new(stage: String, step: usize, total: usize) -> Self {
        Self {
            stage,
            step,
            total,
            completed: 0,
            succeeded: 0,
            failed: 0,
            skipped: 0,
            running: BTreeMap::new(),
            recent: VecDeque::new(),
        }
    }

    /// Consume each start/terminal event once. Only active processes and the
    /// latest 20 terminal rows are retained; full receipts remain on disk.
    pub fn observe(&mut self, outcome: &ProcessOutcome, pid: Option<u32>) {
        let activity = ProcessActivity {
            invocation: outcome.invocation,
            name: outcome
                .inputs
                .first()
                .cloned()
                .unwrap_or_else(|| outcome.stage.clone()),
            input_count: outcome.inputs.len(),
            status: outcome.status,
            pid,
            started_at: outcome.started_at.clone(),
            elapsed_ms: outcome.elapsed_ms,
            error: outcome.error.clone(),
        };
        match outcome.status {
            ProcessStatus::Pending => {}
            ProcessStatus::Running => {
                self.running.insert(outcome.invocation, activity);
            }
            status => {
                self.running.remove(&outcome.invocation);
                self.completed += 1;
                match status {
                    ProcessStatus::Succeeded => self.succeeded += 1,
                    ProcessStatus::Skipped => self.skipped += 1,
                    _ => self.failed += 1,
                }
                self.recent.push_front(activity);
                self.recent.truncate(20);
            }
        }
    }
}
