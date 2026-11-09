//! Versioned corpus identities and read-only verification of extracted originals.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Deserialize)]
struct Profile {
    version: u32,
    id: String,
    name: String,
    archive: Archive,
    workbooks: Vec<Workbook>,
}

#[derive(Deserialize, serde::Serialize)]
struct Archive {
    url: String,
    sha256: String,
    bytes: u64,
}

#[derive(Deserialize)]
struct Workbook {
    index: usize,
    name: String,
    bytes: u64,
    sha256: String,
}

fn profile(id: &str) -> Result<Profile> {
    let source = match id {
        "destatis" => include_str!("../../profiles/corpora/destatis.json"),
        "datagov" => include_str!("../../profiles/corpora/datagov.json"),
        _ => anyhow::bail!("Unknown versioned corpus profile: {id}"),
    };
    let profile: Profile = serde_json::from_str(source)?;
    ensure!(
        profile.version == 1 && profile.id == id,
        "Invalid corpus profile identity"
    );
    ensure!(!profile.name.trim().is_empty(), "Missing corpus display name");
    let url: axum::http::Uri = profile.archive.url.parse()?;
    ensure!(
        url.scheme_str() == Some("https") && url.host().is_some(),
        "Corpus archive requires an absolute HTTPS URL"
    );
    ensure!(
        profile.archive.bytes > 0 && valid_sha256(&profile.archive.sha256),
        "Invalid corpus archive identity"
    );
    let mut names = BTreeSet::new();
    let mut indices = BTreeSet::new();
    for workbook in &profile.workbooks {
        crate::validation_sources::relative_name(&workbook.name)?;
        ensure!(
            names.insert(workbook.name.to_lowercase()) && indices.insert(workbook.index),
            "Duplicate corpus member identity"
        );
        ensure!(
            valid_sha256(&workbook.sha256),
            "Invalid workbook checksum"
        );
    }
    ensure!(!profile.workbooks.is_empty(), "Empty corpus profile");
    Ok(profile)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Expose the embedded profile identities without reading local files or downloading.
pub(crate) fn catalog() -> Result<Value> {
    let mut profiles = Vec::new();
    for id in ["datagov", "destatis"] {
        let profile = profile(id)?;
        profiles.push(json!({
            "id": profile.id,
            "name": profile.name,
            "archive": profile.archive,
            "workbook_count": profile.workbooks.len(),
            "workbook_bytes": profile.workbooks.iter().map(|item| item.bytes).sum::<u64>(),
        }));
    }
    Ok(json!({"version": 1, "profiles": profiles}))
}

/// Report missing or changed inputs without downloading, replacing, or starting Excel.
pub(crate) fn inspect(id: &str, artifacts: &Path) -> Result<Value> {
    inspect_profile(profile(id)?, artifacts)
}

fn inspect_profile(profile: Profile, artifacts: &Path) -> Result<Value> {
    let mut verified = 0;
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    let canonical_root = artifacts
        .exists()
        .then(|| fs::canonicalize(artifacts))
        .transpose()?;
    for workbook in &profile.workbooks {
        let path = artifacts.join(&workbook.name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(&workbook.name);
                continue;
            }
            Err(error) => return Err(error).with_context(|| format!("Inspect {}", path.display())),
        };
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Corpus artifact must be a regular file: {}",
            path.display()
        );
        let canonical = fs::canonicalize(&path)?;
        ensure!(
            canonical_root
                .as_ref()
                .is_some_and(|root| canonical.starts_with(root)),
            "Corpus artifact escapes its root: {}",
            path.display()
        );
        if metadata.len() == workbook.bytes
            && deltasieve_engine::reuse::file_sha256(&path)? == workbook.sha256
        {
            verified += 1;
        } else {
            changed.push(&workbook.name);
        }
    }
    Ok(
        json!({"profile":profile.id,"expected":profile.workbooks.len(),
        "missing_count":missing.len(),"changed_count":changed.len(),
        "verified":verified,"missing":missing,"changed":changed,
        "complete":verified == profile.workbooks.len()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_have_the_expected_unique_workbook_identities() -> Result<()> {
        assert_eq!(profile("destatis")?.workbooks.len(), 1182);
        assert_eq!(profile("datagov")?.workbooks.len(), 9117);
        assert!(profile("unknown").is_err());
        let catalog = catalog()?;
        assert_eq!(catalog["profiles"][0]["id"], "datagov");
        assert_eq!(catalog["profiles"][0]["workbook_count"], 9117);
        assert_eq!(catalog["profiles"][1]["workbook_count"], 1182);
        assert!(catalog["profiles"][0]["archive"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://"));
        assert!(catalog["profiles"][0].get("workbooks").is_none());
        Ok(())
    }

    #[test]
    fn distinguishes_missing_equal_and_changed_files_without_replacing_them() -> Result<()> {
        let root = std::env::temp_dir().join(format!("corpus-inspect-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        fs::write(root.join("equal.xlsx"), b"same")?;
        fs::write(root.join("changed.xlsx"), b"else")?;
        let hash = deltasieve_engine::reuse::file_sha256(&root.join("equal.xlsx"))?;
        let profile = Profile {
            version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            archive: Archive {
                url: "https://example.org/fixture.zip".into(),
                sha256: hash.clone(),
                bytes: 4,
            },
            workbooks: ["equal.xlsx", "missing.xlsx", "changed.xlsx"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| Workbook {
                    index,
                    name: name.into(),
                    bytes: 4,
                    sha256: hash.clone(),
                })
                .collect(),
        };
        let result = inspect_profile(profile, &root)?;
        assert_eq!(result["verified"], 1);
        assert_eq!(result["missing"], json!(["missing.xlsx"]));
        assert_eq!(result["changed"], json!(["changed.xlsx"]));
        assert_eq!(result["complete"], false);
        assert_eq!(fs::read(root.join("changed.xlsx"))?, b"else");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
