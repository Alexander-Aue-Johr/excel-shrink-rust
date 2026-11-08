//! Own temporary work and stage manifests independently of the final visual report.

use super::{Output, RunConfig};
use crate::regression::write_atomic_artifact;
use deltasieve_engine::reuse::file_sha256;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_atomic_artifact(path, &serde_json::to_vec_pretty(value)?)
}

pub(super) fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("Read {}", path.display()))
}

pub(super) fn hash_json(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

/// Canonicalise the existing ancestor before comparing locations of new destinations.
pub(super) fn destination(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::ParentDir)),
        "Use a destination without '..': {}",
        path.display()
    );
    let mut ancestor = path.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("Destination has no existing ancestor")?,
        );
        ancestor = ancestor.parent().context("Destination has no parent")?;
    }
    let mut resolved = fs::canonicalize(ancestor)?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

pub(super) fn validate_locations(config: &RunConfig) -> Result<()> {
    let mut destinations = vec![
        destination(&config.report_dir)?,
        destination(&config.cache_dir)?,
    ];
    if let Some(cache) = &config.stage_cache_dir {
        destinations.push(destination(cache)?);
    }
    if let Output::Directory { path } | Output::Archive { path } = &config.output {
        destinations.push(destination(path)?);
    }
    for (index, path) in destinations.iter().enumerate() {
        for other in destinations.iter().skip(index + 1) {
            ensure!(
                !path.starts_with(other) && !other.starts_with(path),
                "Keep report, cache and processing destinations separate: {} / {}",
                path.display(),
                other.display()
            );
        }
        for source in std::iter::once(&config.source_a).chain(config.source_b.iter()) {
            let source = Path::new(source);
            if source.is_dir() {
                let root = fs::canonicalize(source)?;
                ensure!(
                    !path.starts_with(&root),
                    "An output/cache cannot be inside input source {}",
                    root.display()
                );
            }
        }
    }
    Ok(())
}

/// Only this run's newly created directories may be automatically removed.
pub(super) struct WorkDirectory {
    pub path: PathBuf,
    additional: Vec<PathBuf>,
    preserve: bool,
}

impl WorkDirectory {
    pub fn create(report: &Path) -> Result<Self> {
        let path = report.join("work");
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            additional: Vec::new(),
            preserve: false,
        })
    }
    pub fn track(&mut self, path: PathBuf) {
        self.additional.push(path);
    }
    pub fn preserve(&mut self) {
        self.preserve = true;
    }
    pub fn clean(&mut self) -> Result<()> {
        if self.preserve {
            return Ok(());
        }
        for path in &self.additional {
            if path.exists() {
                fs::remove_dir_all(path)?;
            }
        }
        if self.path.exists() {
            fs::remove_dir_all(&self.path)?;
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct CachedFile {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Deserialize, Serialize)]
pub(super) struct StageManifest {
    pub schema: u32,
    pub key: String,
    pub files: BTreeMap<String, CachedFile>,
    pub sidecars: BTreeMap<String, CachedFile>,
}

impl StageManifest {
    pub fn create(key: String, root: &Path, workbooks: &BTreeMap<String, PathBuf>) -> Result<Self> {
        let mut sidecars = output_fingerprints(root)?;
        let mut files = BTreeMap::new();
        for (name, path) in workbooks {
            ensure!(
                fs::canonicalize(path)? == confined_file(root, name)?,
                "Processor workbook does not match its relative output path: {name}"
            );
            let digest = sidecars
                .remove(name)
                .context("Processor output missing from cache snapshot")?;
            files.insert(name.clone(), digest);
        }
        Ok(Self {
            schema: 2,
            key,
            files,
            sidecars,
        })
    }

    /// A success marker alone is insufficient: every cached output must still match.
    pub fn verify(&self, key: &str, root: &Path) -> Result<BTreeMap<String, PathBuf>> {
        ensure!(
            self.schema == 2 && self.key == key,
            "Stage cache identity differs"
        );
        let root = fs::canonicalize(root)?;
        let mut actual = output_fingerprints(&root)?;
        for (name, recorded) in self.files.iter().chain(&self.sidecars) {
            ensure!(
                actual.remove(name).as_ref() == Some(recorded),
                "Cached processor output changed or is missing: {name}"
            );
        }
        ensure!(
            actual.is_empty(),
            "Unexpected files were added to the processor cache"
        );
        let mut files = BTreeMap::new();
        for name in self.files.keys() {
            let path = confined_file(&root, name)?;
            files.insert(name.clone(), path);
        }
        ensure!(!files.is_empty(), "Empty processor cache");
        Ok(files)
    }
}

/// A later directory stage can inspect sidecars as well as accepted workbooks.
fn output_fingerprints(root: &Path) -> Result<BTreeMap<String, CachedFile>> {
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        crate::validation_sources::require_regular_file(entry.path())?;
        let name = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(
            name,
            CachedFile {
                bytes: entry.metadata()?.len(),
                sha256: file_sha256(entry.path())?,
            },
        );
    }
    Ok(files)
}

/// Accept only ordinary relative files beneath an existing evidence/cache directory.
pub(super) fn confined_file(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    ensure!(
        !relative.contains(['\\', ':'])
            && !relative.chars().any(char::is_control)
            && !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Invalid relative evidence path: {relative}"
    );
    let resolved = fs::canonicalize(root.join(path))?;
    ensure!(
        resolved.starts_with(fs::canonicalize(root)?) && resolved.is_file(),
        "Evidence path escapes its report: {relative}"
    );
    Ok(resolved)
}

pub(super) struct CacheLock(PathBuf);
impl CacheLock {
    pub fn acquire(path: PathBuf) -> Result<Self> {
        fs::create_dir(&path).with_context(|| format!(
            "Stage cache is in use or interrupted: {}. Remove this lock only after its owner has stopped",
            path.display()))?;
        let lock = Self(path);
        fs::write(
            lock.0.join("owner.json"),
            serde_json::to_vec(&serde_json::json!({
                "pid": std::process::id(), "started_at": chrono::Utc::now().to_rfc3339()
            }))?,
        )?;
        Ok(lock)
    }
}
impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_cache_output_cannot_be_reused() -> Result<()> {
        let root = std::env::temp_dir().join(format!("sheet-scope-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let file = root.join("book.xlsx");
        fs::write(&file, b"old")?;
        let manifest = StageManifest::create(
            "key".into(),
            &root,
            &BTreeMap::from([("book.xlsx".into(), file.clone())]),
        )?;
        assert!(manifest.verify("key", &root).is_ok());
        fs::write(file, b"new")?;
        assert!(manifest.verify("key", &root).is_err());
        assert!(manifest.verify("different", &root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn sidecar_changes_and_unexpected_files_invalidate_a_stage_cache() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("sheet-scope-sidecars-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let workbook = root.join("book.xlsx");
        let sidecar = root.join("conversion.json");
        fs::write(&workbook, b"workbook")?;
        fs::write(&sidecar, b"original metadata")?;
        let manifest = StageManifest::create(
            "key".into(),
            &root,
            &BTreeMap::from([("book.xlsx".into(), workbook)]),
        )?;
        assert_eq!(manifest.verify("key", &root)?.len(), 1);
        fs::write(&sidecar, b"changed metadata")?;
        assert!(manifest.verify("key", &root).is_err());
        fs::write(&sidecar, b"original metadata")?;
        fs::write(root.join("unexpected.txt"), b"new")?;
        assert!(manifest.verify("key", &root).is_err());
        fs::remove_file(root.join("unexpected.txt"))?;
        fs::remove_file(sidecar)?;
        assert!(manifest.verify("key", &root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
