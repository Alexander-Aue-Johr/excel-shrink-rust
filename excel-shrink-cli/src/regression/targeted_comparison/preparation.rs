//! Generate the same Excel controls without rendering or a visual verdict.
use super::{
    telemetry::{LaneId, Phase, StepStatus},
    Comparison,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

impl Comparison<'_> {
    pub(super) fn prepare_artifacts(&mut self) -> Result<()> {
        self.start_excel_host()?;
        let include_default = !self.report.input_identity.identical();
        let original = self.paths.original.clone();
        let default = self.paths.default.clone();
        self.copy_control(&original, "original.before-save.xlsx")?;
        if include_default {
            self.copy_control(&default, "default.before-save.xlsx")?;
        }
        // Artifact generation needs only one live workbook per worker. Keeping
        // independent controls open together consumes RAM without adding evidence.
        for (lane, raw, saved) in [
            (LaneId::O, self.args.original.clone(), original.clone()),
            (LaneId::SD, self.args.default.clone(), default.clone()),
        ] {
            if lane == LaneId::SD && !include_default {
                continue;
            }
            self.open_workbook(lane)?;
            self.telemetry
                .update(|state| state.step(lane, Phase::Save, StepStatus::Running))?;
            self.excel
                .as_mut()
                .context("Excel host missing")?
                .save_workbook(lane, &self.telemetry)?;
            self.telemetry
                .update(|state| state.step(lane, Phase::Save, StepStatus::Done))?;
            self.remember_control(lane, &raw, &[("saved", &saved)])?;
            self.close_workbook(lane)?;
        }
        self.optimize_performance_copy()?;
        if !self.args.skip_check_performance {
            self.close_workbook(LaneId::CP)?;
        }
        if include_default {
            self.prepare_control_rebuild(LaneId::SDR, &default)?;
        }
        if let Some(cp) = self.performance.path.clone() {
            self.prepare_control_rebuild(LaneId::CPR, &cp)?;
            self.prepare_save_as_control(&original)?;
        }
        self.prepare_control_rebuild(LaneId::OR, &original)?;
        self.report.status = "artifacts_prepared".into();
        self.report
            .phases
            .push("Excel artifact generation complete; visual validation pending".into());
        let operations = &self
            .excel
            .as_ref()
            .context("Excel host missing")?
            .operation_measurements;
        fs::write(
            self.args.output.join("operations.json"),
            serde_json::to_vec_pretty(operations)?,
        )?;
        fs::write(
            self.args.output.join("preparation.json"),
            serde_json::to_vec_pretty(&json!({
                "status":"artifacts_prepared", "visual_validation":"pending",
                "original":self.args.original, "default":self.args.default,
                "check_performance":self.report.check_performance,
                "check_performance_cache":self.report.check_performance_cache,
                "control_artifacts_cache":self.report.control_artifacts_cache,
                "input_identity":self.report.input_identity,
                "default_state":if include_default {"observed"} else {"skipped_identical_input"},
                "copy_timings_ms":self.report.timings_ms,
                "artifact_bytes":self.report.workbook_bytes,
                "operations":operations
            }))?,
        )?;
        self.release_excel_and_wait_for_exit()?;
        Ok(())
    }

    fn copy_control(&mut self, source: &Path, name: &str) -> Result<PathBuf> {
        let destination = self.args.output.join(name);
        let started = Instant::now();
        let bytes = fs::copy(source, &destination)?;
        self.report
            .timings_ms
            .insert(format!("copy:{name}"), started.elapsed().as_millis() as u64);
        self.report.workbook_bytes.insert(name.into(), bytes);
        Ok(destination)
    }

    fn prepare_control_rebuild(&mut self, lane: LaneId, parent: &Path) -> Result<()> {
        let after = self
            .args
            .output
            .join(format!("{}-rebuilt.xlsx", lane.key()));
        let before = after.with_extension("before-rebuild.xlsx");
        let cached = if lane == LaneId::CPR {
            self.restore_performance_rebuild(parent, &before, &after)?
        } else {
            self.restore_control(lane, parent, &before, &after)?
        };
        if cached {
            return Ok(());
        }
        let after = self.copy_control(parent, &format!("{}-rebuilt.xlsx", lane.key()))?;
        let host = self.excel.as_mut().context("Excel host missing")?;
        host.open_workbook(lane, &after, &self.telemetry)?;
        host.save_workbook(lane, &self.telemetry)?;
        self.copy_control(
            &after,
            &format!("{}-rebuilt.before-rebuild.xlsx", lane.key()),
        )?;
        self.excel
            .as_mut()
            .unwrap()
            .recalculate_workbook(lane, &self.telemetry)?;
        if lane == LaneId::CPR {
            self.remember_performance_artifacts(Some((parent, &before, &after)));
        } else {
            self.remember_control(
                lane,
                parent,
                &[("before_rebuild", &before), ("after_rebuild", &after)],
            )?;
        }
        self.close_workbook(lane)?;
        Ok(())
    }

    fn prepare_save_as_control(&mut self, original: &Path) -> Result<()> {
        let saved_as = self
            .args
            .output
            .join("check-performance.optimized.save-as.xlsx");
        let rebuilt = self
            .args
            .output
            .join("check-performance.optimized.save-as.rebuilt.saved.xlsx");
        let baseline = self
            .args
            .output
            .join("check-performance.save-as.before-optimization.xlsx");
        let environment = self
            .excel
            .as_mut()
            .context("Excel host missing")?
            .cache_environment(&self.telemetry)?;
        let cache = super::control_cache::Store::open(
            &self.args.original,
            original,
            "CP-save-as",
            environment,
        )?;
        if super::performance_dependencies::reusable_rebuild(original)? {
            if let Ok(Some(historical)) = cache.restore_named(&[
                ("before_optimization", &baseline),
                ("after_save_as", &saved_as),
                ("after_rebuild", &rebuilt),
            ]) {
                self.report.control_artifacts_cache["CP-save-as"] =
                    json!({"status":"hit", "historical":historical});
                return Ok(());
            }
        }
        let working = self.copy_control(original, "check-performance.save-as-working.xlsx")?;
        let host = self.excel.as_mut().context("Excel host missing")?;
        host.open_workbook(LaneId::CP, &working, &self.telemetry)?;
        host.save_workbook(LaneId::CP, &self.telemetry)?;
        self.copy_control(
            &working,
            "check-performance.save-as.before-optimization.xlsx",
        )?;
        let native = self
            .excel
            .as_mut()
            .unwrap()
            .optimize_workbook(&self.telemetry)?;
        fs::write(
            self.args
                .output
                .join("check-performance.save-as.native.json"),
            serde_json::to_vec_pretty(&native)?,
        )?;
        self.excel
            .as_mut()
            .unwrap()
            .save_as_workbook(LaneId::CP, &saved_as, &self.telemetry)?;
        self.excel
            .as_mut()
            .unwrap()
            .recalculate_workbook(LaneId::CP, &self.telemetry)?;
        self.copy_control(
            &working,
            "check-performance.optimized.save-as.rebuilt.saved.xlsx",
        )?;
        let measurements = self
            .excel
            .as_mut()
            .unwrap()
            .measurements(LaneId::CP, &self.telemetry)?;
        let historical = cache.publish(
            &[
                ("before_optimization", &baseline),
                ("after_save_as", &saved_as),
                ("after_rebuild", &rebuilt),
            ],
            json!({"native_result":native,"measurements":measurements}),
        )?;
        self.report.control_artifacts_cache["CP-save-as"] =
            json!({"status":"generated", "historical":historical});
        self.close_workbook(LaneId::CP)?;
        Ok(())
    }
}
