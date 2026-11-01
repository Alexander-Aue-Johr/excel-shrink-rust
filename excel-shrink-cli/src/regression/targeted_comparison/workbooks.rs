//! Working copies and explicit lifecycle operations for the Excel sessions.
use super::host::ExcelHost;
use super::telemetry::{LaneId, Lifecycle, Phase, StepStatus};
use super::{Args, Comparison, WorkingPaths};
use crate::native_process::process_alive;
use anyhow::{ensure, Context, Result};
use std::{fs, time::Duration};

impl WorkingPaths {
    /// Validate inputs and create isolated copies before any Excel process starts.
    pub(super) fn prepare(args: &Args) -> Result<Self> {
        ensure!(
            !args.output.exists(),
            "Output directory already exists: {}",
            args.output.display()
        );
        ensure!(
            args.original.is_file() && args.default.is_file(),
            "Both XLSX inputs must exist"
        );
        fs::create_dir_all(&args.output)?;
        let paths = WorkingPaths {
            original: args.output.join("original.xlsx"),
            default: args.output.join("default.xlsx"),
        };
        fs::copy(&args.original, &paths.original).context("Copy original")?;
        fs::copy(&args.default, &paths.default).context("Copy shrink result")?;
        Ok(paths)
    }

    /// Preserve each workbook's state immediately before Excel saves its copy.
    pub(super) fn preserve_before_save(&self) -> Result<Self> {
        let original = self.original.with_extension("before-save.xlsx");
        let default = self.default.with_extension("before-save.xlsx");
        fs::copy(&self.original, &original)?;
        fs::copy(&self.default, &default)?;
        Ok(Self { original, default })
    }
}

impl Comparison<'_> {
    /// Start the command host once for all workbook lifecycles.
    pub(super) fn start_excel_host(&mut self) -> Result<()> {
        self.excel = Some(ExcelHost::start(
            self.args.host_script.as_deref(),
            Duration::from_secs(self.args.operation_timeout_seconds),
        )?);
        Ok(())
    }

    /// Publish opening, record the confirmed PID, and publish readiness in order.
    pub(super) fn open_workbook(&mut self, id: LaneId) -> Result<()> {
        let path = match id {
            LaneId::O => &self.paths.original,
            LaneId::SD => &self.paths.default,
            LaneId::CP => self
                .performance
                .path
                .as_ref()
                .context("Check Performance copy missing")?,
        };
        self.telemetry.update(|state| {
            state.step(id, Phase::Open, StepStatus::Running);
            state.lane(id).lifecycle = Lifecycle::Opening;
        })?;
        let pid = self
            .excel
            .as_mut()
            .context("Excel host not started")?
            .open_workbook(id, path, &self.telemetry)?;
        self.report.excel_pids.insert(id.key().into(), pid);
        self.telemetry.update(|state| {
            state.lane(id).lifecycle = Lifecycle::Open;
            state.step(id, Phase::Open, StepStatus::Done);
        })?;
        Ok(())
    }

    /// Save Original before Default, preserving the existing event ordering.
    pub(super) fn save_workbooks(&mut self) -> Result<()> {
        for id in [LaneId::O, LaneId::SD] {
            self.telemetry
                .update(|state| state.step(id, Phase::Save, StepStatus::Running))?;
            self.excel
                .as_mut()
                .context("Excel host not started")?
                .save_workbook(id, &self.telemetry)?;
            self.telemetry
                .update(|state| state.step(id, Phase::Save, StepStatus::Done))?;
        }
        Ok(())
    }

    /// Publish one rebuild's actual start and successful completion.
    pub(super) fn rebuild(&mut self, id: LaneId) -> Result<()> {
        self.telemetry
            .update(|state| state.step(id, Phase::Recalculate, StepStatus::Running))?;
        self.excel
            .as_mut()
            .context("Excel host not started")?
            .recalculate_workbook(id, &self.telemetry)?;
        self.telemetry
            .update(|state| state.step(id, Phase::Recalculate, StepStatus::Done))
    }

    /// Mark a workbook closed only after the host confirms its process exit.
    pub(super) fn close_workbook(&mut self, id: LaneId) -> Result<()> {
        self.telemetry.update(|state| {
            state.lane(id).lifecycle = Lifecycle::Closing;
            state.step(id, Phase::Close, StepStatus::Running);
        })?;
        self.excel
            .as_mut()
            .context("Excel host not started")?
            .close_workbook(id, &self.telemetry)?;
        self.telemetry.update(|state| {
            let lane = state.lane(id);
            lane.lifecycle = Lifecycle::Closed;
            lane.process_alive = Some(false);
            lane.closed_at = Some(chrono::Utc::now().timestamp_millis());
            state.step(id, Phase::Close, StepStatus::Done);
        })?;
        Ok(())
    }

    /// Release the owned host job and retain the existing bounded exit wait.
    pub(super) fn release_excel_and_wait_for_exit(&mut self) -> Result<()> {
        self.excel.take();
        let identities = self.telemetry.owned_processes()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while identities
            .iter()
            .any(|(pid, started)| process_alive(i64::from(*pid), *started) != Some(false))
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}
