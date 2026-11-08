//! Resume into a new batch without changing earlier native artifacts or attribution.
use super::*;
use std::collections::BTreeMap;

pub(super) struct Index {
    entries: BTreeMap<String, Entry>,
}

pub(super) struct Entry {
    directory: PathBuf,
    receipt: Value,
    receipt_sha256: String,
    producer: Option<String>,
}

impl Index {
    /// Only a stopped pool or a positively departed owner can supply continuation artifacts.
    pub fn open(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root)?;
        if read_json(&root.join("workers/status.json"))
            .is_none_or(|status| status["stopped"] != true)
        {
            let owner = read_json(&root.join("owner.json"))
                .context("Previous preparation has no verified stopped pool or owner identity")?;
            let pid = owner["pid"]
                .as_i64()
                .context("Previous owner PID missing")?;
            let started = owner["started_ms"]
                .as_i64()
                .context("Previous owner creation time missing")?;
            anyhow::ensure!(
                crate::native_process::process_alive(pid, started) == Some(false),
                "Previous preparation owner is still alive or its exit cannot be verified"
            );
        }
        let batch_producer = read_json(&root.join("preparation.json"))
            .and_then(|batch| batch["producer_sha256"].as_str().map(str::to_owned));
        let mut entries = BTreeMap::new();
        for directory in fs::read_dir(&root)? {
            let directory = directory?;
            if !directory
                .file_name()
                .to_string_lossy()
                .starts_with("input-")
            {
                continue;
            }
            let kind = directory.file_type()?;
            anyhow::ensure!(
                kind.is_dir() && !kind.is_symlink(),
                "Previous input is not an owned directory"
            );
            let directory = fs::canonicalize(directory.path())?;
            anyhow::ensure!(
                directory.starts_with(&root),
                "Previous input escapes its preparation root"
            );
            let path = directory.join("prepared.json");
            if !path.try_exists()? {
                continue;
            }
            let kind = fs::symlink_metadata(&path)?;
            anyhow::ensure!(
                kind.is_file()
                    && !kind.is_symlink()
                    && fs::canonicalize(&path)?.starts_with(&directory),
                "Previous receipt escapes its input directory"
            );
            let bytes = fs::read(&path)?;
            let receipt: Value = serde_json::from_slice(&bytes)?;
            let id = receipt["source_id"]
                .as_str()
                .context("Previous prepared source ID missing")?
                .to_owned();
            let producer = match receipt.get("producer_sha256") {
                Some(value) => Some(
                    value
                        .as_str()
                        .context("Invalid prior per-input producer")?
                        .to_owned(),
                ),
                None => batch_producer.clone(),
            };
            let entry = Entry {
                directory,
                receipt,
                receipt_sha256: super::super::super::artifact_cache::sha256(&bytes),
                producer,
            };
            anyhow::ensure!(
                entries.insert(id, entry).is_none(),
                "Previous batch contains ambiguous source occurrences"
            );
        }
        Ok(Self { entries })
    }

    pub fn lookup(&self, source: &Input, recipe: &Value) -> Result<Option<&Entry>> {
        let Some(entry) = self.entries.get(&source.source_id) else {
            return Ok(None);
        };
        let receipt = &entry.receipt;
        anyhow::ensure!(
            receipt["version"] == 1 && receipt["operation"] == "check-performance",
            "Unsupported prior preparation receipt"
        );
        anyhow::ensure!(
            receipt["generation_recipe"] == *recipe,
            "Native preparation recipe changed"
        );
        anyhow::ensure!(
            receipt["artifacts"]
                .as_object()
                .is_some_and(|artifacts| artifacts.len()
                    == validation_preparation::PREPARATION_CONTROLS.len()),
            "Unsupported previous artifact set"
        );
        anyhow::ensure!(
            matches!(
                receipt["native_result"]["status"].as_str(),
                Some("optimization_confirmed" | "no_optimization_needed")
            ),
            "Previous native completion is unconfirmed"
        );
        let actual = super::super::super::artifact_cache::hash_file(&source.baseline)?;
        anyhow::ensure!(
            receipt["source_sha256"] == actual
                && source
                    .expected_sha256
                    .as_ref()
                    .is_none_or(|expected| expected == &actual),
            "Prepared source baseline changed"
        );
        if let Some(raw) = &source.raw_source {
            anyhow::ensure!(
                receipt["raw_source"] == serde_json::to_value(raw)?,
                "Raw-source provenance changed"
            );
        }
        validation_preparation::normalized_sha(
            entry
                .producer
                .as_deref()
                .context("Previous producer identity unavailable")?,
        )?;
        anyhow::ensure!(
            receipt["baseline"]["file"] == "baseline.xlsx"
                && receipt["baseline"]["sha256"] == actual,
            "Previous retained baseline is unbound"
        );
        for name in
            std::iter::once("baseline.xlsx").chain(validation_preparation::PREPARATION_CONTROLS)
        {
            entry.verify_file(name)?;
        }
        Ok(Some(entry))
    }
}

impl Entry {
    fn descriptor(&self, name: &str) -> &Value {
        if name == "baseline.xlsx" {
            &self.receipt["baseline"]
        } else {
            &self.receipt["artifacts"][name]
        }
    }

    fn verify_file(&self, name: &str) -> Result<PathBuf> {
        let path = self.directory.join(name);
        let kind = fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            kind.is_file() && !kind.is_symlink(),
            "Previous artifact is not a regular file"
        );
        let path = fs::canonicalize(path)?;
        anyhow::ensure!(
            path.starts_with(&self.directory),
            "Previous artifact escapes its input directory"
        );
        let descriptor = self.descriptor(name);
        anyhow::ensure!(
            descriptor["bytes"].as_u64() == Some(fs::metadata(&path)?.len()),
            "Previous artifact length changed"
        );
        anyhow::ensure!(
            descriptor["sha256"] == super::super::super::artifact_cache::hash_file(&path)?,
            "Previous artifact hash changed"
        );
        Ok(path)
    }

    /// Copy verified states into a new directory, preserving original metrics and producer.
    pub fn restore(&self, destination: &Path) -> Result<()> {
        fs::create_dir(destination)?;
        for name in
            std::iter::once("baseline.xlsx").chain(validation_preparation::PREPARATION_CONTROLS)
        {
            let source = self.verify_file(name)?;
            let target = destination.join(name);
            fs::copy(source, &target)?;
            anyhow::ensure!(
                self.descriptor(name)["sha256"]
                    == super::super::super::artifact_cache::hash_file(&target)?,
                "Previous artifact changed during continuation copy"
            );
        }
        let mut receipt = self.receipt.clone();
        receipt["producer_sha256"] = json!(self.producer);
        receipt["reuse"] =
            json!({"source_directory":self.directory,"source_receipt_sha256":self.receipt_sha256});
        write_json(&destination.join("prepared.json"), &receipt)
    }
}

#[cfg(test)]
pub(super) fn fixture() -> Result<(PathBuf, PathBuf, PathBuf, Value)> {
    let root =
        std::env::temp_dir().join(format!("performance-continuation-{}", uuid::Uuid::new_v4()));
    let previous = root.join("previous");
    let directory = previous.join("input-000000");
    fs::create_dir_all(&directory)?;
    let pool = Pool::new_performance(previous.join("workers"))?;
    let recipe = generation_recipe(&pool.root)?;
    drop(pool);
    let source = root.join("source.xlsx");
    write_seed(&source)?;
    let hash = super::super::super::artifact_cache::hash_file(&source)?;
    let bytes = fs::metadata(&source)?.len();
    let mut artifacts = BTreeMap::new();
    fs::copy(&source, directory.join("baseline.xlsx"))?;
    for name in validation_preparation::PREPARATION_CONTROLS {
        fs::copy(&source, directory.join(name))?;
        artifacts.insert(name, json!({"bytes":bytes,"sha256":hash}));
    }
    write_json(
        &directory.join("prepared.json"),
        &json!({
            "version":1,"operation":"check-performance","source_id":"stable-source","name":"source.xlsx",
            "source":source,"source_sha256":hash,"producer_sha256":"a".repeat(64),
            "baseline":{"file":"baseline.xlsx","bytes":bytes,"sha256":hash},"artifacts":artifacts,
            "native_result":{"status":"optimization_confirmed","method":"unit-fixture"},
            "environment":{"machine":"unit-producer"},"generation_recipe":recipe,"measurements":{"open_seconds":12.5}
        }),
    )?;
    let manifest = root.join("sources.json");
    write_json(
        &manifest,
        &json!({"version":1,"inputs":[{
            "source_id":"stable-source","name":"source.xlsx","baseline":source,"baseline_sha256":hash
        }]}),
    )?;
    Ok((root, previous, manifest, recipe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_preserves_old_artifacts_producer_and_measurements() -> Result<()> {
        let (root, previous, manifest, recipe) = fixture()?;
        let input = validation_preparation::read(&manifest)?.remove(0);
        let before = fs::read(previous.join("input-000000/prepared.json"))?;
        let index = Index::open(&previous)?;
        index
            .lookup(&input, &recipe)?
            .unwrap()
            .restore(&root.join("restored"))?;
        assert_eq!(
            before,
            fs::read(previous.join("input-000000/prepared.json"))?
        );
        let restored = read_json(&root.join("restored/prepared.json")).unwrap();
        assert_eq!(restored["producer_sha256"], "a".repeat(64));
        assert_eq!(restored["measurements"]["open_seconds"], 12.5);
        assert!(restored["reuse"]["source_receipt_sha256"].is_string());
        Ok(())
    }

    #[test]
    fn incompatible_recipe_changed_baseline_and_corrupt_artifacts_are_rejected() -> Result<()> {
        let (_, previous, manifest, recipe) = fixture()?;
        let input = validation_preparation::read(&manifest)?.remove(0);
        let index = Index::open(&previous)?;
        assert!(index.lookup(&input, &json!({"changed":"recipe"})).is_err());
        fs::write(
            previous.join("input-000000/after-optimization.xlsx"),
            b"corrupt",
        )?;
        assert!(index.lookup(&input, &recipe).is_err());
        fs::copy(
            &input.baseline,
            previous.join("input-000000/after-optimization.xlsx"),
        )?;
        fs::write(&input.baseline, b"changed")?;
        assert!(index.lookup(&input, &recipe).is_err());
        Ok(())
    }

    #[test]
    fn a_live_owner_cannot_be_used_for_continuation() -> Result<()> {
        let (_, previous, _, _) = fixture()?;
        write_json(
            &previous.join("workers/status.json"),
            &json!({"stopped":false}),
        )?;
        write_json(
            &previous.join("owner.json"),
            &json!({"pid":std::process::id(),"started_ms":crate::native_process::current_started_unix_ms()}),
        )?;
        assert!(Index::open(&previous).is_err());
        Ok(())
    }
}
