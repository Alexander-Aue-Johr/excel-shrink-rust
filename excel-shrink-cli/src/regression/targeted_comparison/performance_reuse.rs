//! Cache lifecycle is separate from native optimization and visual comparison.
use super::{
    performance_cache::{self, Cache},
    telemetry::{LaneId, Phase, StepStatus},
    Comparison,
};
use crate::regression::artifact_cache;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

impl Comparison<'_> {
    /// Cache failure is explicit but cannot invalidate otherwise valid fresh evidence.
    pub(super) fn prepare_performance_cache(&mut self) -> Result<()> {
        self.telemetry.update(|state| {
            state.step(LaneId::CP, Phase::Discover, StepStatus::Running);
            state.lane(LaneId::CP).decision_note =
                Some("Checking Check Performance artifact cache".into());
        })?;
        let result = (|| -> Result<Cache> {
            let environment = self
                .excel
                .as_mut()
                .context("Excel host missing")?
                .cache_environment(&self.telemetry)?;
            let root = match std::env::var_os("EXCEL_SHRINK_CHECK_PERFORMANCE_CACHE_DIR") {
                Some(path) => PathBuf::from(path),
                None => artifact_cache::root()?.join("check-performance-v1"),
            };
            Cache::open(root, &self.args.original, &self.paths.original, environment)
        })();
        match result {
            Ok(cache) => {
                self.performance.cache = Some(cache);
                self.refresh_cache_provenance();
            }
            Err(error) => self.cache_warning(&error),
        }
        self.telemetry.update(|state| {
            state.step(LaneId::CP, Phase::Discover, StepStatus::Done);
            if self
                .performance
                .cache
                .as_ref()
                .is_none_or(|cache| cache.entry.is_none())
            {
                state.lane(LaneId::CP).decision_note =
                    Some("No reusable Check Performance cache — generating native result".into());
            }
        })?;
        Ok(())
    }

    /// Restore owned copies and observe their new opening/save effects as candidates.
    pub(super) fn restore_performance_cache(&mut self) -> Result<bool> {
        let Some(cache) = self
            .performance
            .cache
            .as_ref()
            .filter(|c| c.entry.is_some())
        else {
            return Ok(false);
        };
        for (name, destination) in [
            ("original_saved", "check-performance.cached-original.xlsx"),
            (
                "before_optimization",
                "check-performance.before-optimization.xlsx",
            ),
            (
                "after_optimization",
                "check-performance.after-optimization.xlsx",
            ),
            ("after_optimization", "check-performance.xlsx"),
        ] {
            cache.restore(name, &self.args.output.join(destination))?;
        }
        let native = cache.entry.as_ref().unwrap().native_result.clone();
        self.performance.cache_hit = true;
        self.performance.path = Some(self.args.output.join("check-performance.xlsx"));
        self.telemetry.update(|state| {
            state.lane(LaneId::CP).decision_note = Some(
                "Check Performance from cache — historical native outcome and measurements".into(),
            );
            state.step(LaneId::CP, Phase::Optimize, StepStatus::Skipped);
        })?;
        self.open_workbook(LaneId::CP)?;
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Save, StepStatus::Running))?;
        self.excel
            .as_mut()
            .context("Excel host missing")?
            .save_workbook(LaneId::CP, &self.telemetry)?;
        self.telemetry
            .update(|state| state.step(LaneId::CP, Phase::Save, StepStatus::Done))?;
        self.adopt_cached_performance_evidence(native)?;
        self.refresh_cache_provenance();
        Ok(true)
    }

    /// A cached rebuild is usable only for the same current parent contents.
    pub(super) fn restore_performance_rebuild(
        &mut self,
        source: &Path,
        before: &Path,
        path: &Path,
    ) -> Result<bool> {
        let Some(cache) = self
            .performance
            .cache
            .as_ref()
            .filter(|c| c.entry.as_ref().is_some_and(|e| e.rebuild_parent.is_some()))
        else {
            return Ok(false);
        };
        if cache.entry.as_ref().unwrap().rebuild_parent.as_deref()
            != Some(performance_cache::workbook_fingerprint(source)?.as_str())
        {
            return Ok(false);
        }
        if !super::performance_dependencies::reusable_rebuild(source)? {
            self.report.check_performance_cache["rebuild_lookup"] =
                json!("fresh rebuild required: volatile or unsupported dependencies");
            return Ok(false);
        }
        cache.restore("before_rebuild", before)?;
        cache.restore("after_rebuild", path)?;
        cache.restore(
            "after_rebuild",
            &path.with_extension("cached-after-rebuild.xlsx"),
        )?;
        self.performance.cached_rebuild = true;
        self.report.check_performance_cache["rebuild_lookup"] =
            json!("hit; recognized deterministic dependencies and matching parent contents");
        self.refresh_cache_provenance();
        Ok(true)
    }

    /// Preserve native artifacts as soon as they exist, even if later captures fail.
    pub(super) fn remember_performance_artifacts(
        &mut self,
        rebuild: Option<(&Path, &Path, &Path)>,
    ) {
        let result = (|| -> Result<()> {
            if self.performance.cache.is_none() {
                return Ok(());
            }
            let id = if rebuild.is_some() {
                LaneId::CPR
            } else {
                LaneId::CP
            };
            let sample = self
                .excel
                .as_mut()
                .context("Excel host missing")?
                .measurements(id, &self.telemetry)?;
            self.performance
                .measurements
                .insert(id.key().into(), sample);
            let cache = self.performance.cache.as_mut().unwrap();
            let mut measurements = cache
                .entry
                .as_ref()
                .map(|e| e.measurements.clone())
                .unwrap_or_default();
            measurements.extend(self.performance.measurements.clone());
            let mut files = Vec::new();
            if cache.entry.is_none() {
                files.extend([
                    ("original_saved", self.paths.original.clone()),
                    (
                        "before_optimization",
                        self.args
                            .output
                            .join("check-performance.before-optimization.xlsx"),
                    ),
                    (
                        "after_optimization",
                        self.args
                            .output
                            .join("check-performance.after-optimization.xlsx"),
                    ),
                ]);
            }
            let parent = if let Some((source, before, after)) = rebuild {
                files.extend([
                    ("before_rebuild", before.to_owned()),
                    ("after_rebuild", after.to_owned()),
                ]);
                Some(performance_cache::workbook_fingerprint(source)?)
            } else {
                cache.entry.as_ref().and_then(|e| e.rebuild_parent.clone())
            };
            let native = self
                .report
                .check_performance_evidence
                .as_ref()
                .context("Missing native confirmation")?
                .native_result
                .clone();
            cache.publish(&files, native, measurements, parent)
        })();
        if let Err(error) = result {
            self.cache_warning(&error);
        } else {
            self.refresh_cache_provenance();
        }
    }

    /// A cache reopen measurement is new; the generation measurements stay historical.
    pub(super) fn retain_performance_measurements(&mut self, id: LaneId) {
        if !matches!(id, LaneId::CP | LaneId::CPR) || self.performance.cache.is_none() {
            return;
        }
        let result = self
            .excel
            .as_mut()
            .context("Excel host missing")
            .and_then(|host| host.measurements(id, &self.telemetry));
        match result {
            Ok(sample)
                if self.performance.cache_hit
                    && (id == LaneId::CP || self.performance.cached_rebuild) =>
            {
                self.report.check_performance_cache["current_open_measurements"][id.key()] = sample;
            }
            Ok(sample) => {
                self.performance
                    .measurements
                    .insert(id.key().into(), sample);
                let cache = self.performance.cache.as_mut().unwrap();
                if let Some(entry) = cache.entry.clone() {
                    let mut measurements = entry.measurements;
                    measurements.extend(self.performance.measurements.clone());
                    if let Err(error) =
                        cache.publish(&[], entry.native_result, measurements, entry.rebuild_parent)
                    {
                        self.cache_warning(&error);
                    } else {
                        self.refresh_cache_provenance();
                    }
                }
            }
            Err(error) => self.cache_warning(&error),
        }
    }

    fn refresh_cache_provenance(&mut self) {
        if let Some(cache) = &self.performance.cache {
            let current = self.report.check_performance_cache["current_open_measurements"].clone();
            let warning = self.report.check_performance_cache["warning"].clone();
            let rebuild_lookup = self.report.check_performance_cache["rebuild_lookup"].clone();
            self.report.check_performance_cache = cache.provenance(self.performance.cache_hit);
            self.report.check_performance_cache["rebuild_reused"] =
                json!(self.performance.cached_rebuild);
            self.report.check_performance_cache["current_open_measurements"] = current;
            self.report.check_performance_cache["warning"] = warning;
            self.report.check_performance_cache["rebuild_lookup"] = rebuild_lookup;
        }
    }

    fn cache_warning(&mut self, error: &anyhow::Error) {
        eprintln!("[targeted] Check Performance cache unavailable: {error:#}");
        if self.report.check_performance_cache.is_null() {
            self.report.check_performance_cache = json!({"status":"unavailable"});
        }
        self.report.check_performance_cache["warning"] = Value::String(format!("{error:#}"));
    }
}
