//! Freeze existing XLSX directories as preparation inputs without modifying originals.

use super::*;

pub(crate) fn directory_inputs(root: &Path, profile_id: Option<&str>) -> Result<Vec<Input>> {
    crate::validation_sources::require_regular_source(root)?;
    let root = fs::canonicalize(root)?;
    ensure!(
        root.is_dir(),
        "Choose a directory containing XLSX originals"
    );
    if let Some(id) = profile_id {
        let profile = crate::validation_corpora::profile(id)?;
        let inspection = crate::validation_corpora::inspect(id, &root)?;
        ensure!(
            inspection["complete"] == true,
            "Corpus originals are incomplete or changed ({} missing, {} changed)",
            inspection["missing_count"],
            inspection["changed_count"]
        );
        return profile
            .workbooks
            .into_iter()
            .map(|member| {
                Ok(Input {
                    source_id: format!(
                        "{}:{}:{}",
                        profile.archive.sha256, member.index, member.name
                    ),
                    baseline: resolve_xlsx(&root.join(&member.name))?,
                    name: member.name.clone(),
                    expected_sha256: Some(member.sha256.clone()),
                    raw_source: Some(RawSource {
                        sha256: member.sha256,
                        format: "xlsx".into(),
                        member: Some(CorpusMember {
                            corpus_id: profile.archive.sha256.clone(),
                            index: member.index as u64,
                            name: member.name,
                        }),
                    }),
                })
            })
            .collect();
    }
    let source = crate::validation_sources::discover(&root)?;
    ensure!(
        !source.files.is_empty(),
        "No workbook inputs in the selected directory"
    );
    source
        .files
        .into_iter()
        .map(|(name, path)| {
            let baseline = resolve_xlsx(&path)?;
            let sha256 = crate::validation_sources::hash_file(&baseline)?;
            Ok(Input {
                source_id: format!("directory:{name}"),
                name,
                baseline,
                expected_sha256: Some(sha256.clone()),
                raw_source: Some(RawSource {
                    sha256,
                    format: "xlsx".into(),
                    member: None,
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_inputs_keep_relative_occurrences_and_reject_unconverted_workbooks() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("preparation-directory-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("nested"))?;
        fs::write(root.join("a.xlsx"), b"same")?;
        fs::write(root.join("nested/a.xlsx"), b"same")?;
        let inputs = directory_inputs(&root, None)?;
        assert_eq!(inputs.len(), 2);
        assert_ne!(inputs[0].source_id, inputs[1].source_id);
        assert_eq!(inputs[0].expected_sha256, inputs[1].expected_sha256);
        assert_eq!(inputs[1].name, "nested/a.xlsx");
        assert!(inputs[0].raw_source.as_ref().unwrap().member.is_none());
        fs::write(root.join("legacy.xls"), b"legacy")?;
        let error = directory_inputs(&root, None)
            .err()
            .context("Expected legacy-format rejection")?;
        assert!(error.to_string().contains("must use XLSX"));
        assert!(directory_inputs(&root, Some("destatis")).is_err());
        assert_eq!(fs::read(root.join("a.xlsx"))?, b"same");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
