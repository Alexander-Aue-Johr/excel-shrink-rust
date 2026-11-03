//! Reusable ordinary-save and rebuild controls, with exact generation provenance.
use super::{performance_cache, performance_dependencies, Comparison};
use crate::regression::artifact_cache;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const POLICY: &str = "excel-control-artifacts-v1";

pub(super) struct Store {
    directory: PathBuf,
    identity: Value,
}

impl Store {
    pub fn open(raw: &Path, parent: &Path, lane: &str, environment: Value) -> Result<Self> {
        let identity = json!({"policy": POLICY, "lane": lane,
            "raw_sha256": artifact_cache::hash_file(raw)?,
            "parent_fingerprint": performance_cache::workbook_fingerprint(parent)?,
            "environment": environment});
        let root = std::env::var_os("EXCEL_SHRINK_CONTROL_ARTIFACT_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or(artifact_cache::root()?.join("excel-controls-v1"));
        let key = artifact_cache::sha256(&serde_json::to_vec(&identity)?);
        Ok(Self {
            directory: root.join(key),
            identity,
        })
    }

    pub fn restore(&self, before: &Path, after: &Path) -> Result<Option<Value>> {
        self.restore_named(&[("before_rebuild", before), ("after_rebuild", after)])
    }

    pub fn restore_named(&self, files: &[(&str, &Path)]) -> Result<Option<Value>> {
        let manifest = self.directory.join("manifest.json");
        if !manifest.is_file() {
            return Ok(None);
        }
        let entry: Value = serde_json::from_slice(&fs::read(manifest)?)?;
        ensure!(
            entry["identity"] == self.identity,
            "Control generation identity differs"
        );
        if self.identity["lane"] == "CP-save-as" {
            ensure!(
                matches!(
                    entry["measurements"]["native_result"]["status"].as_str(),
                    Some("optimization_confirmed" | "no_optimization_needed")
                ),
                "Missing Save As optimization confirmation"
            );
        }
        for (name, destination) in files {
            let artifact = &entry["artifacts"][*name];
            let sha = artifact["sha256"]
                .as_str()
                .context("Missing control checksum")?;
            ensure!(
                sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid control checksum"
            );
            let source = self.directory.join(format!("{sha}.xlsx"));
            ensure!(
                fs::metadata(&source)?.len()
                    == artifact["bytes"].as_u64().context("Missing control size")?,
                "Control size differs"
            );
            ensure!(
                artifact_cache::hash_file(&source)? == sha,
                "Control checksum differs"
            );
            fs::copy(&source, destination)?;
            ensure!(
                artifact_cache::hash_file(destination)? == sha,
                "Restored control checksum differs"
            );
        }
        Ok(Some(entry))
    }

    pub fn publish(&self, files: &[(&str, &Path)], measurements: Value) -> Result<Value> {
        fs::create_dir_all(&self.directory)?;
        let mut artifacts = serde_json::Map::new();
        for (name, source) in files {
            let sha = artifact_cache::hash_file(source)?;
            let destination = self.directory.join(format!("{sha}.xlsx"));
            if !destination.is_file() {
                let temporary = artifact_cache::unique_path(&self.directory, "control")?;
                fs::copy(source, &temporary)?;
                fs::OpenOptions::new()
                    .write(true)
                    .open(&temporary)?
                    .sync_all()?;
                if let Err(error) = fs::rename(&temporary, &destination) {
                    if !destination.is_file() {
                        return Err(error.into());
                    }
                    fs::remove_file(&temporary)?;
                }
            }
            ensure!(
                artifact_cache::hash_file(&destination)? == sha,
                "Published control checksum differs"
            );
            artifacts.insert(
                (*name).into(),
                json!({"sha256": sha, "bytes": fs::metadata(source)?.len()}),
            );
        }
        let entry = json!({"identity": self.identity, "artifacts": artifacts,
            "created_at": chrono::Utc::now().to_rfc3339(), "producer_sha256": artifact_cache::hash_file(&std::env::current_exe()?)?,
            "measurements": measurements});
        artifact_cache::atomic_write(
            &self.directory.join("manifest.json"),
            &serde_json::to_vec(&entry)?,
        )?;
        Ok(entry)
    }
}

impl Comparison<'_> {
    fn control_store(&mut self, lane: super::telemetry::LaneId, parent: &Path) -> Result<Store> {
        use super::telemetry::LaneId;
        let environment = self
            .excel
            .as_mut()
            .context("Excel host missing")?
            .cache_environment(&self.telemetry)?;
        let raw = if matches!(lane, LaneId::SD | LaneId::SDR) {
            &self.args.default
        } else {
            &self.args.original
        };
        Store::open(raw, parent, lane.key(), environment)
    }

    pub(super) fn restore_control(
        &mut self,
        lane: super::telemetry::LaneId,
        parent: &Path,
        before: &Path,
        after: &Path,
    ) -> Result<bool> {
        if !performance_dependencies::reusable_rebuild(parent)? {
            return Ok(false);
        }
        let result = self.control_store(lane, parent)?.restore(before, after);
        match result {
            Ok(Some(historical)) => {
                self.report.control_artifacts_cache[lane.key()] = json!({"status":"hit", "historical":historical,
                    "current_parent_sha256":artifact_cache::hash_file(parent)?});
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(error) => {
                self.report.control_artifacts_cache[lane.key()] =
                    json!({"status":"unavailable", "error":format!("{error:#}")});
                Ok(false)
            }
        }
    }

    pub(super) fn remember_control(
        &mut self,
        lane: super::telemetry::LaneId,
        parent: &Path,
        files: &[(&str, &Path)],
    ) -> Result<()> {
        let measurements = self
            .excel
            .as_mut()
            .context("Excel host missing")?
            .measurements(lane, &self.telemetry)?;
        let store = self.control_store(lane, parent)?;
        let historical = store.publish(files, measurements)?;
        self.report.control_artifacts_cache[lane.key()] =
            json!({"status":"generated", "historical":historical});
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Result<(PathBuf, Store, PathBuf)> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "excel-control-test")?;
        fs::create_dir_all(&root)?;
        let source = root.join("source.xlsx");
        fs::write(&source, b"immutable fixture bytes")?;
        let store = Store {
            directory: root.join("cache"),
            identity: json!({"policy":POLICY,"lane":"OR","parent":"first"}),
        };
        Ok((root, store, source))
    }

    #[test]
    fn controls_restore_owned_copies_and_preserve_measurements() -> Result<()> {
        let (root, store, source) = fixture()?;
        store.publish(
            &[("before_rebuild", &source), ("after_rebuild", &source)],
            json!({"private_bytes":123}),
        )?;
        let before = root.join("before.xlsx");
        let after = root.join("after.xlsx");
        let entry = store.restore(&before, &after)?.unwrap();
        assert_eq!(entry["measurements"]["private_bytes"], 123);
        fs::write(&after, b"changed working copy")?;
        store.restore(&before, &after)?;
        assert_eq!(fs::read(&after)?, fs::read(&source)?);
        Ok(())
    }

    #[test]
    fn corrupt_or_mismatched_generation_is_rejected() -> Result<()> {
        let (root, store, source) = fixture()?;
        let entry = store.publish(
            &[("before_rebuild", &source), ("after_rebuild", &source)],
            Value::Null,
        )?;
        let before = root.join("before.xlsx");
        let after = root.join("after.xlsx");
        let mut changed = entry.clone();
        changed["identity"]["parent"] = json!("different");
        fs::write(
            store.directory.join("manifest.json"),
            serde_json::to_vec(&changed)?,
        )?;
        assert!(store.restore(&before, &after).is_err());
        fs::write(
            store.directory.join("manifest.json"),
            serde_json::to_vec(&entry)?,
        )?;
        let hash = entry["artifacts"]["after_rebuild"]["sha256"]
            .as_str()
            .unwrap();
        fs::write(store.directory.join(format!("{hash}.xlsx")), b"corrupt")?;
        assert!(store.restore(&before, &after).is_err());
        Ok(())
    }

    #[test]
    fn incomplete_or_unsafe_control_manifest_is_rejected() -> Result<()> {
        let (root, store, source) = fixture()?;
        let mut entry = store.publish(&[("before_rebuild", &source)], Value::Null)?;
        assert!(store
            .restore(&root.join("before"), &root.join("after"))
            .is_err());
        entry["artifacts"]["before_rebuild"]["sha256"] = json!("../../unowned.xlsx");
        fs::write(
            store.directory.join("manifest.json"),
            serde_json::to_vec(&entry)?,
        )?;
        assert!(store
            .restore(&root.join("before"), &root.join("after"))
            .is_err());
        Ok(())
    }
}
