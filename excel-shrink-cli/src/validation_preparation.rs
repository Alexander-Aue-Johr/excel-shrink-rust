//! Stable source occurrences shared by preparation and portable artifact matching.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
pub(crate) mod package;
pub(crate) mod imported;
use std::io::BufRead;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Member indices preserve repeated archive names as distinct occurrences.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusMember {
    pub corpus_id: String,
    pub index: u64,
    pub name: String,
}

/// Source identity before an optional XLS conversion produced the baseline.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawSource {
    pub sha256: String,
    pub format: String,
    pub member: Option<CorpusMember>,
}

/// Display names remain metadata and never choose output filesystem paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestInput {
    pub source_id: String,
    pub name: String,
    pub baseline: PathBuf,
    pub baseline_sha256: String,
    pub raw_source: Option<RawSource>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    inputs: Vec<ManifestInput>,
}

/// Local paths are resolved once; expected content is checked at job admission.
pub(crate) struct Input {
    pub source_id: String,
    pub name: String,
    pub baseline: PathBuf,
    pub expected_sha256: Option<String>,
    pub raw_source: Option<RawSource>,
}

/// Resolve occurrence identities without opening Excel or reading all baseline contents.
pub(crate) fn read(path: &Path) -> Result<Vec<Input>> {
    let path = fs::canonicalize(path)?;
    let mut reader = std::io::BufReader::new(fs::File::open(&path)?);
    if reader.fill_buf()?.starts_with(&[0xef, 0xbb, 0xbf]) {
        reader.consume(3);
    }
    let manifest: Manifest = serde_json::from_reader(reader)?;
    ensure!(
        manifest.version == 1,
        "Unsupported preparation source manifest version"
    );
    ensure!(
        !manifest.inputs.is_empty(),
        "The preparation source manifest is empty"
    );
    let parent = path.parent().context("Source manifest has no parent")?;
    let mut ids = BTreeSet::new();
    manifest
        .inputs
        .into_iter()
        .map(|mut item| {
            ensure!(
                !item.source_id.is_empty() && !item.source_id.chars().any(char::is_control),
                "Preparation source ID must be nonempty and contain no control characters"
            );
            ensure!(
                ids.insert(item.source_id.clone()),
                "Duplicate preparation source ID: {}",
                item.source_id
            );
            ensure!(
                !item.name.is_empty() && !item.name.chars().any(char::is_control),
                "Invalid preparation display name"
            );
            item.baseline_sha256 = normalized_sha(&item.baseline_sha256)?;
            if let Some(source) = &mut item.raw_source {
                source.sha256 = normalized_sha(&source.sha256)?;
                ensure!(!source.format.is_empty(), "Raw source format is missing");
                if let Some(member) = &source.member {
                    ensure!(
                        !member.corpus_id.is_empty() && !member.name.is_empty(),
                        "Incomplete corpus member reference"
                    );
                }
            }
            let baseline = if item.baseline.is_relative() {
                parent.join(item.baseline)
            } else {
                item.baseline
            };
            Ok(Input {
                source_id: item.source_id,
                name: item.name,
                baseline: resolve_xlsx(&baseline)?,
                expected_sha256: Some(item.baseline_sha256),
                raw_source: item.raw_source,
            })
        })
        .collect()
}

/// Explicit lists retain occurrence identity without claiming a corpus mapping.
pub(crate) fn explicit(paths: &[PathBuf]) -> Result<Vec<Input>> {
    ensure!(!paths.is_empty(), "Choose at least one prepared XLSX input");
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let baseline = resolve_xlsx(path)?;
            let path_key =
                crate::regression::artifact_cache::sha256(baseline.to_string_lossy().as_bytes());
            Ok(Input {
                source_id: format!("explicit:{index}:{path_key}"),
                name: baseline
                    .file_name()
                    .context("Input has no filename")?
                    .to_string_lossy()
                    .into_owned(),
                baseline,
                expected_sha256: None,
                raw_source: None,
            })
        })
        .collect()
}

/// Bind selected IDs independently of queue order and worker concurrency.
pub(crate) fn selection_hash(inputs: &[Input]) -> Result<String> {
    let mut ids: Vec<_> = inputs.iter().map(|input| &input.source_id).collect();
    ids.sort_unstable();
    Ok(crate::regression::artifact_cache::sha256(
        &serde_json::to_vec(&ids)?,
    ))
}

fn resolve_xlsx(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    ensure!(
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx")),
        "Preparation baseline paths must use XLSX"
    );
    match fs::canonicalize(&path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error.into()),
    }
}

fn normalized_sha(value: &str) -> Result<String> {
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid SHA-256 identity"
    );
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("preparation-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("baseline.xlsx"), b"baseline").unwrap();
        root
    }

    #[test]
    fn resolves_relative_baselines_and_preserves_raw_occurrences() -> Result<()> {
        let root = fixture();
        let item = json!({"source_id":"corpus:4:book.xls","name":"book.xls","baseline":"baseline.xlsx",
            "baseline_sha256":"A".repeat(64),"raw_source":{"sha256":"B".repeat(64),"format":"xls",
            "member":{"corpus_id":"corpus","index":4,"name":"book.xls"}}});
        let path = root.join("sources.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({"version":1,"inputs":[item]}))?,
        )?;
        let inputs = read(&path)?;
        assert_eq!(
            inputs[0].baseline,
            fs::canonicalize(root.join("baseline.xlsx"))?
        );
        assert_eq!(inputs[0].expected_sha256, Some("a".repeat(64)));
        assert_eq!(
            inputs[0]
                .raw_source
                .as_ref()
                .unwrap()
                .member
                .as_ref()
                .unwrap()
                .index,
            4
        );
        Ok(())
    }

    #[test]
    fn rejects_duplicate_ids_and_unknown_versions() -> Result<()> {
        let root = fixture();
        let item = json!({"source_id":"same","name":"book","baseline":"baseline.xlsx", "baseline_sha256":"a".repeat(64)});
        let path = root.join("sources.json");
        for manifest in [
            json!({"version":1,"inputs":[item.clone(),item.clone()]}),
            json!({"version":2,"inputs":[item]}),
        ] {
            fs::write(&path, serde_json::to_vec(&manifest)?)?;
            assert!(read(&path).is_err());
        }
        Ok(())
    }

    #[test]
    fn explicit_duplicates_keep_occurrences_and_selection_is_order_independent() -> Result<()> {
        let root = fixture();
        let mut inputs = explicit(&[root.join("baseline.xlsx"), root.join("baseline.xlsx")])?;
        assert_ne!(inputs[0].source_id, inputs[1].source_id);
        let hash = selection_hash(&inputs)?;
        inputs.reverse();
        assert_eq!(hash, selection_hash(&inputs)?);
        Ok(())
    }

    #[test]
    fn missing_baselines_remain_selected_for_per_input_failure_reporting() -> Result<()> {
        let root = fixture();
        let inputs = explicit(&[root.join("missing.xlsx"), root.join("baseline.xlsx")])?;
        assert_eq!(inputs.len(), 2);
        assert!(!inputs[0].baseline.exists());
        assert!(inputs[1].baseline.is_file());
        Ok(())
    }

    #[test]
    fn accepts_windows_utf8_bom_and_parent_relative_baselines() -> Result<()> {
        let root = fixture();
        fs::create_dir(root.join("config"))?;
        let manifest = json!({"version":1,"inputs":[{
            "source_id":"one", "name":"baseline", "baseline":"../baseline.xlsx",
            "baseline_sha256":"a".repeat(64)
        }]});
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend(serde_json::to_vec(&manifest)?);
        let path = root.join("config/sources.json");
        fs::write(&path, bytes)?;
        let inputs = read(&path)?;
        assert_eq!(fs::read(&inputs[0].baseline)?, b"baseline");
        Ok(())
    }
}
