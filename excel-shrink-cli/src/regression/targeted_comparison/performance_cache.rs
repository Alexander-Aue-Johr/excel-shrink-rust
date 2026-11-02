//! Immutable Check Performance workbooks; publication never certifies visual equality.
use crate::regression::{artifact_cache, xml_views};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const POLICY: &str = "check-performance-artifacts-v1";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Artifact {
    file: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    policy: String,
    key: String,
    pub created_at: String,
    pub producer: String,
    pub environment: Value,
    identity: Value,
    pub native_result: Value,
    pub artifacts: BTreeMap<String, Artifact>,
    pub measurements: BTreeMap<String, Value>,
    pub rebuild_parent: Option<String>,
}

pub(super) struct Cache {
    root: PathBuf,
    key: String,
    environment: Value,
    identity: Value,
    pub entry: Option<Entry>,
    pub lookup: String,
}

impl Cache {
    /// Bind reuse to source, saved Original, Excel environment and generation recipe,
    /// deliberately excluding the shrinker binary and the visual comparison policy.
    pub fn open(root: PathBuf, original: &Path, saved: &Path, environment: Value) -> Result<Self> {
        ensure!(
            environment["excel_sha256"]
                .as_str()
                .is_some_and(|s| s.len() == 64)
                && environment["excel_version"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
            "Excel environment is not verified"
        );
        let identity = serde_json::json!({
            "policy": POLICY, "original": artifact_cache::hash_file(original)?,
            "saved_original": workbook_fingerprint(saved)?, "environment": environment
        });
        let key = artifact_cache::sha256(&serde_json::to_vec(&identity)?);
        let mut cache = Self {
            root,
            key,
            environment,
            identity,
            entry: None,
            lookup: "miss".into(),
        };
        let path = cache.manifest_path();
        if path.exists() {
            match cache.read_entry(&path) {
                Ok(entry) => {
                    cache.entry = Some(entry);
                    cache.lookup = "hit".into();
                }
                Err(error) => cache.lookup = format!("invalid entry; regenerating: {error:#}"),
            }
        }
        Ok(cache)
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join(format!("{}.json", self.key))
    }

    fn read_entry(&self, path: &Path) -> Result<Entry> {
        ensure!(
            fs::metadata(path)?.len() <= 1024 * 1024,
            "Oversized cache manifest"
        );
        let entry: Entry = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            entry.policy == POLICY
                && entry.key == self.key
                && entry.environment == self.environment
                && entry.identity == self.identity
                && entry.artifacts.len() <= 5,
            "Cache identity mismatch"
        );
        ensure!(
            matches!(
                entry.native_result["status"].as_str(),
                Some("optimization_confirmed" | "no_optimization_needed")
            ),
            "Unconfirmed native outcome"
        );
        for name in [
            "original_saved",
            "before_optimization",
            "after_optimization",
        ] {
            ensure!(entry.artifacts.contains_key(name), "Missing {name}");
        }
        ensure!(
            entry.artifacts.contains_key("before_rebuild")
                == entry.artifacts.contains_key("after_rebuild")
                && entry.rebuild_parent.is_some() == entry.artifacts.contains_key("after_rebuild"),
            "Partial rebuild cache"
        );
        for artifact in entry.artifacts.values() {
            let path = self.artifact_path(artifact)?;
            ensure!(
                fs::metadata(&path)?.len() == artifact.bytes
                    && artifact_cache::hash_file(&path)? == artifact.sha256,
                "Corrupt cached workbook"
            );
        }
        ensure!(
            entry.measurements.contains_key("CP"),
            "Missing historical CP measurements"
        );
        if entry.rebuild_parent.is_some() {
            ensure!(
                entry.measurements.contains_key("CPR"),
                "Missing historical rebuild measurements"
            );
        }
        Ok(entry)
    }

    fn artifact_path(&self, artifact: &Artifact) -> Result<PathBuf> {
        // The manifest cannot escape this cache, including through a replaced blob symlink.
        let name = artifact
            .file
            .strip_suffix(".xlsx")
            .context("Invalid cache filename")?;
        ensure!(
            name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid cache object name"
        );
        let blobs = fs::canonicalize(self.root.join("blobs"))?;
        let path = fs::canonicalize(blobs.join(&artifact.file))?;
        ensure!(
            path.parent() == Some(blobs.as_path()),
            "Cache object escapes blob directory"
        );
        Ok(path)
    }

    /// Copy a verified immutable artifact into this attempt; Excel never opens cache files.
    pub fn restore(&self, name: &str, destination: &Path) -> Result<()> {
        let artifact = self
            .entry
            .as_ref()
            .context("No cache entry")?
            .artifacts
            .get(name)
            .with_context(|| format!("Missing cached {name}"))?;
        fs::copy(self.artifact_path(artifact)?, destination)?;
        ensure!(
            artifact_cache::hash_file(destination)? == artifact.sha256,
            "Cache changed during restore"
        );
        Ok(())
    }

    /// Publish only complete immutable copies, with one atomic manifest replacement.
    /// Concurrent writers can choose different complete generations, never mix their files.
    pub fn publish(
        &mut self,
        files: &[(&str, PathBuf)],
        native: Value,
        measurements: BTreeMap<String, Value>,
        rebuild_parent: Option<String>,
    ) -> Result<()> {
        let blobs = self.root.join("blobs");
        fs::create_dir_all(&blobs)?;
        let mut artifacts = self
            .entry
            .as_ref()
            .map(|e| e.artifacts.clone())
            .unwrap_or_default();
        for (name, source) in files {
            let sha256 = artifact_cache::hash_file(source)?;
            let existing = artifacts.values().find(|a| a.sha256 == sha256).cloned();
            let artifact = if let Some(existing) = existing {
                existing
            } else {
                let file = format!("{}.xlsx", uuid::Uuid::new_v4().simple());
                let destination = blobs.join(&file);
                fs::copy(source, &destination)?;
                fs::OpenOptions::new()
                    .write(true)
                    .open(&destination)?
                    .sync_all()?;
                ensure!(
                    artifact_cache::hash_file(&destination)? == sha256,
                    "Workbook changed during cache publication"
                );
                Artifact {
                    file,
                    sha256,
                    bytes: fs::metadata(&destination)?.len(),
                }
            };
            artifacts.insert((*name).into(), artifact);
        }
        let entry = Entry {
            policy: POLICY.into(),
            key: self.key.clone(),
            created_at: self
                .entry
                .as_ref()
                .map(|e| e.created_at.clone())
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            producer: self
                .entry
                .as_ref()
                .map(|e| e.producer.clone())
                .unwrap_or(artifact_cache::hash_file(&std::env::current_exe()?)?),
            environment: self.environment.clone(),
            identity: self.identity.clone(),
            native_result: native,
            artifacts,
            measurements,
            rebuild_parent,
        };
        // Apply the same completeness checks before making a generation discoverable.
        let pending = artifact_cache::unique_path(&self.root, "performance-entry")?;
        fs::write(&pending, serde_json::to_vec(&entry)?)?;
        let validated = self.read_entry(&pending);
        let _ = fs::remove_file(&pending);
        let entry = validated?;
        artifact_cache::atomic_write(&self.manifest_path(), &serde_json::to_vec(&entry)?)?;
        self.entry = Some(entry);
        Ok(())
    }

    pub fn provenance(&self, reused: bool) -> Value {
        serde_json::json!({"status":if reused {"hit"} else if self.entry.is_some() {"generated"} else {"miss"},
            "lookup":self.lookup,"key":self.key,"policy":POLICY,
            "manifest":self.manifest_path(),"historical":self.entry,
            "measurements_are_historical":reused})
    }
}

/// Hash actual uncompressed member contents, not ZIP CRCs or compression details.
/// Only core/app properties and recognized workbook path/revision metadata are
/// non-computational noise. Formula caches, drawings, styles and all other parts count.
pub(super) fn workbook_fingerprint(path: &Path) -> Result<String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut parts = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() || matches!(entry.name(), "docProps/core.xml" | "docProps/app.xml") {
            continue;
        }
        let mut hash = Sha256::new();
        if entry.name() == "xl/workbook.xml" {
            let mut xml = Vec::new();
            entry.read_to_end(&mut xml)?;
            hash.update(normalize_workbook(&xml)?);
        } else {
            let mut block = [0; 128 * 1024];
            loop {
                let count = entry.read(&mut block)?;
                if count == 0 {
                    break;
                }
                hash.update(&block[..count]);
            }
        }
        ensure!(
            parts
                .insert(entry.name().to_owned(), format!("{:x}", hash.finalize()))
                .is_none(),
            "Duplicate package part"
        );
    }
    Ok(artifact_cache::sha256(&serde_json::to_vec(&parts)?))
}

fn normalize_workbook(xml: &[u8]) -> Result<Vec<u8>> {
    let mut reader = xml_views::Reader::<128>::new(xml)?;
    let mut metadata = super::static_workbook_metadata::WorkbookMetadata::default();
    let mut normalized = Vec::new();
    let mut position = 0;
    while let Some(tag) = reader.next()? {
        let known = metadata.accepts(tag)?;
        if known
            && tag.kind == xml_views::Kind::Empty
            && matches!(tag.name, b"x15ac:absPath" | b"xr:revisionPtr")
        {
            normalized.extend_from_slice(&xml[position..tag.start]);
            // Keep the tag identity; discard only values of a recognized static form.
            normalized.extend_from_slice(tag.name);
            position = tag.end;
        }
    }
    normalized.extend_from_slice(&xml[position..]);
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn environment() -> Value {
        json!({"excel_sha256":"a".repeat(64),"excel_version":"16.0.test","generation_recipe":"recipe-one"})
    }

    fn workbook(path: &Path, value: &str, metadata: &str, style: &str) -> Result<()> {
        let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
        for (name, content) in [
            ("xl/workbook.xml", "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets/></workbook>"),
            ("xl/worksheets/sheet1.xml", value),
            ("xl/styles.xml", style),
            ("docProps/core.xml", metadata),
        ] {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(content.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    }

    fn publish(cache: &mut Cache, source: &Path) -> Result<()> {
        let files = [
            "original_saved",
            "before_optimization",
            "after_optimization",
        ]
        .map(|name| (name, source.to_owned()));
        cache.publish(
            &files,
            json!({"status":"no_optimization_needed"}),
            [(
                "CP".into(),
                json!({"startup_seconds":2,"peak_private_bytes":123,"method":"fixture"}),
            )]
            .into(),
            None,
        )
    }

    #[test]
    fn key_binds_source_saved_contents_and_environment_but_not_document_properties() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "performance-key")?;
        fs::create_dir_all(&root)?;
        let original = root.join("input.xlsx");
        let saved = root.join("saved.xlsx");
        workbook(&original, "one", "old", "style")?;
        workbook(&saved, "one", "first save", "style")?;
        let key = Cache::open(root.join("cache"), &original, &saved, environment())?.key;
        workbook(&saved, "one", "later save", "style")?;
        assert_eq!(
            key,
            Cache::open(root.join("cache"), &original, &saved, environment())?.key
        );
        let mut changed_environment = environment();
        changed_environment["generation_recipe"] = json!("new recipe");
        assert_ne!(
            key,
            Cache::open(root.join("cache"), &original, &saved, changed_environment)?.key
        );
        for (value, style) in [
            ("changed cached formula value", "style"),
            ("one", "different style"),
        ] {
            workbook(&saved, value, "later save", style)?;
            assert_ne!(
                key,
                Cache::open(root.join("cache"), &original, &saved, environment())?.key
            );
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn only_known_path_and_revision_metadata_are_normalized() -> Result<()> {
        let xml = br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"><mc:AlternateContent><mc:Choice Requires="x15"><x15ac:absPath url="old-directory" xmlns:x15ac="http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac"/></mc:Choice></mc:AlternateContent><workbookPr date1904="0"/></workbook>"#;
        let changed = String::from_utf8(xml.to_vec())?.replace("old-directory", "new-directory");
        assert_eq!(
            normalize_workbook(xml)?,
            normalize_workbook(changed.as_bytes())?
        );
        let changed = changed.replace("date1904=\"0\"", "date1904=\"1\"");
        assert_ne!(
            normalize_workbook(xml)?,
            normalize_workbook(changed.as_bytes())?
        );
        let unsupported = String::from_utf8(xml.to_vec())?.replace("2010/11/ac", "2099/11/ac");
        let changed = unsupported.replace("old-directory", "new-directory");
        assert_ne!(
            normalize_workbook(unsupported.as_bytes())?,
            normalize_workbook(changed.as_bytes())?
        );
        Ok(())
    }

    #[test]
    fn complete_cache_restores_copies_and_corruption_never_becomes_a_hit() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "performance-cache")?;
        fs::create_dir_all(&root)?;
        let input = root.join("input.xlsx");
        workbook(&input, "one", "metadata", "style")?;
        let mut cache = Cache::open(root.join("cache"), &input, &input, environment())?;
        assert!(cache.entry.is_none());
        publish(&mut cache, &input)?;
        let loaded = Cache::open(root.join("cache"), &input, &input, environment())?;
        assert_eq!(loaded.lookup, "hit");
        loaded.restore("after_optimization", &root.join("copy.xlsx"))?;
        fs::write(
            root.join("copy.xlsx"),
            "Excel may mutate only the owned copy",
        )?;
        assert!(
            Cache::open(root.join("cache"), &input, &input, environment())?
                .entry
                .is_some()
        );
        let artifact = &loaded.entry.as_ref().unwrap().artifacts["after_optimization"];
        fs::write(loaded.artifact_path(artifact)?, "broken cache")?;
        let invalid = Cache::open(root.join("cache"), &input, &input, environment())?;
        assert!(invalid.entry.is_none());
        assert!(invalid.lookup.contains("regenerating"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn incomplete_publication_preserves_the_last_complete_generation() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "performance-transaction")?;
        fs::create_dir_all(&root)?;
        let input = root.join("input.xlsx");
        workbook(&input, "one", "metadata", "style")?;
        let mut cache = Cache::open(root.join("cache"), &input, &input, environment())?;
        publish(&mut cache, &input)?;
        let manifest = fs::read(cache.manifest_path())?;
        assert!(cache
            .publish(
                &[("after_rebuild", input.clone())],
                json!({"status":"no_optimization_needed"}),
                cache.entry.as_ref().unwrap().measurements.clone(),
                Some("parent".into())
            )
            .is_err());
        assert_eq!(manifest, fs::read(cache.manifest_path())?);
        assert!(
            Cache::open(root.join("cache"), &input, &input, environment())?
                .entry
                .is_some()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn concurrent_publication_never_mixes_workbook_generations() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "performance-concurrent")?;
        fs::create_dir_all(&root)?;
        let input = root.join("input.xlsx");
        workbook(&input, "one", "metadata", "style")?;
        let mut threads = Vec::new();
        for marker in ["generation A", "generation B"] {
            let root = root.clone();
            let input = input.clone();
            threads.push(std::thread::spawn(move || -> Result<()> {
                let source = root.join(format!("{marker}.xlsx"));
                fs::write(&source, marker)?;
                let mut cache = Cache::open(root.join("cache"), &input, &input, environment())?;
                publish(&mut cache, &source)
            }));
        }
        for thread in threads {
            thread.join().unwrap()?;
        }
        let cache = Cache::open(root.join("cache"), &input, &input, environment())?;
        let artifacts = &cache.entry.as_ref().unwrap().artifacts;
        assert!(artifacts
            .values()
            .all(|a| a.sha256 == artifacts["after_optimization"].sha256));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
