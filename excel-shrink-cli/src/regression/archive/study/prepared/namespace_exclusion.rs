//! Exclude unsupported XML roots without treating passthrough as visual success.
use super::*;

pub(super) fn skip(job: &MemberJob, record: &mut Value, source: &Path) -> Result<bool> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let Some(part) = crate::pipeline::namespace_passthrough_part(&mut archive)? else {
        return Ok(false);
    };
    let reason = format!(
        "Unsupported XML namespace or root representation in {part}; workbook excluded from shrinking and visual comparison."
    );
    record["namespace_exclusion"] = json!({
        "code":"UNSUPPORTED_XML_ROOT", "part":part,
        "method":"bounded-xml-root-headers-v1",
        "scope":"XML root headers only; no full-document namespace scan"
    });
    record["status"] = json!("skipped");
    record["error"] = json!(reason);
    record["capture_mode"] = json!("namespace-exclusion");
    if record["last_execution"].is_object() {
        record["last_execution"]["capture_mode"] = json!("namespace-exclusion");
    }
    for field in [
        "package_identity",
        "resident_capture",
        "capture_policy",
        "visual_coverage",
    ] {
        record.as_object_mut().unwrap().remove(field);
    }
    // Retained legacy bundles can contain three byte-identical passthroughs.
    // Keep their real preparation measurements, but make no comparison claim.
    for profile in record["results"].as_array_mut().into_iter().flatten() {
        if let Some(fields) = profile.as_object_mut() {
            fields.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "profile"
                        | "original_bytes"
                        | "shrunk_bytes"
                        | "reduction_ratio"
                        | "shrink_seconds"
                        | "config_hash"
                )
            });
            fields.insert("status".into(), json!("not compared; unsupported XML root"));
            fields.insert("checks_complete".into(), json!(false));
            fields.insert("failures".into(), json!([]));
            fields.insert("sheets".into(), json!([]));
        }
    }
    record["whitespace_diagnosis"] = json!({"status":"not_applicable",
        "reason":"workbook excluded because its XML root representation is unsupported"});
    work_state::event(
        "excluded",
        "namespace_excluded",
        &reason,
        true,
        None,
        record["namespace_exclusion"].clone(),
    );
    write_record(&job.record, record)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIXED_SHEET: &[u8] = br#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><s:sheetData/></s:worksheet>"#;

    fn write_workbook(path: &Path, sheet: &[u8]) -> Result<()> {
        let mut archive = zip::ZipWriter::new(fs::File::create(path)?);
        for (name, bytes) in [
            ("xl/workbook.xml", b"<workbook/>".as_slice()),
            ("xl/worksheets/sheet.xml", sheet),
        ] {
            archive.start_file(name, zip::write::FileOptions::default())?;
            archive.write_all(bytes)?;
        }
        archive.finish()?;
        Ok(())
    }

    #[test]
    fn namespace_exclusion_cannot_be_reported_as_unchanged_or_measured() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-exclusion")?;
        let job = super::super::tests::provenance_job(&root);
        let source = root.join("prefixed.xlsx");
        write_workbook(&source, PREFIXED_SHEET)?;
        let before = fs::read(&source)?;
        let mut record = json!({"status":"unchanged","prepared":true,
            "package_identity":{"status":"equivalent"},"last_execution":{"capture_mode":"resident"},
            "results":[{"profile":"default","original_bytes":500,"shrunk_bytes":500,
                "shrink_seconds":0.01,"status":"package unchanged","package_checks_complete":true,
                "checks_complete":true,"original_open_seconds":4.0,"sheets":[{"status":"same"}]}]});
        assert!(skip(&job, &mut record, &source)?);
        assert_eq!(record["status"], "skipped");
        assert_eq!(
            record["namespace_exclusion"]["method"],
            "bounded-xml-root-headers-v1"
        );
        assert_eq!(
            record["namespace_exclusion"]["part"],
            "xl/worksheets/sheet.xml"
        );
        assert!(record["package_identity"].is_null());
        assert_eq!(record["results"][0]["checks_complete"], false);
        assert!(record["results"][0]["package_checks_complete"].is_null());
        assert!(record["results"][0]["original_open_seconds"].is_null());
        assert_eq!(record["results"][0]["shrink_seconds"], 0.01);
        assert_eq!(
            record["prepared"], true,
            "existing bundles remain reusable evidence"
        );
        assert_eq!(fs::read(source)?, before);
        let saved: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
        assert_eq!(saved, record);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn ordinary_xml_is_not_excluded_even_if_it_would_shrink_to_identical_bytes() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-supported")?;
        let job = super::super::tests::provenance_job(&root);
        let source = root.join("ordinary.xlsx");
        write_workbook(&source, br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#)?;
        let mut record = json!({"status":"comparing","results":[]});
        let before = record.clone();
        assert!(!skip(&job, &mut record, &source)?);
        assert_eq!(record, before);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn preparation_excludes_before_baseline_shrink_or_excel_work() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-preparation")?;
        let mut job = super::super::tests::provenance_job(&root);
        job.max_workbook_bytes = 1024 * 1024;
        fs::create_dir_all(&job.work)?;
        let source = root.join("prefixed.xlsx");
        write_workbook(&source, PREFIXED_SHEET)?;
        let mut archive = zip::ZipWriter::new(fs::File::create(&job.archive)?);
        archive.start_file(&job.name, zip::write::FileOptions::default())?;
        archive.write_all(&fs::read(source)?)?;
        archive.finish()?;
        let mut record = json!({"name":job.name});
        super::super::prepare(&job, &mut record, LogLevel::Warn)?;
        assert_eq!(record["status"], "skipped");
        assert_eq!(record["prepared"], false);
        assert!(record["baseline_sha256"].is_null());
        assert_eq!(record["results"], json!([]));
        assert!(!job.bundle.as_ref().unwrap().exists());
        assert!(!job.work.join("default").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn retained_passthrough_bundle_is_excluded_before_package_equivalence() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-retained-bundle")?;
        let mut job = super::super::tests::provenance_job(&root);
        job.mode = JobMode::Compare;
        fs::create_dir_all(&job.work)?;
        let source = root.join("prefixed.xlsx");
        write_workbook(&source, PREFIXED_SHEET)?;
        let input = fs::read(source)?;
        let mut bundle = zip::ZipWriter::new(fs::File::create(job.bundle.as_ref().unwrap())?);
        for name in std::iter::once("original").chain(PROFILES) {
            bundle.start_file(format!("{name}.xlsx"), zip::write::FileOptions::default())?;
            bundle.write_all(&input)?;
        }
        bundle.start_file("measurements.json", zip::write::FileOptions::default())?;
        bundle.write_all(&serde_json::to_vec(
            &PROFILES.map(|name| json!({"profile":name,"shrink_seconds":0.01})),
        )?)?;
        bundle.finish()?;
        // The original bundle has no baseline fingerprint. The old package-audit
        // shortcut would fail or claim preservation, neither is this exclusion.
        let mut record = json!({"prepared":true,"name":job.name});
        super::super::compare(&job, &mut record)?;
        assert_eq!(record["status"], "skipped");
        assert_eq!(record["capture_mode"], "namespace-exclusion");
        assert!(record["package_identity"].is_null());
        assert!(job.bundle.as_ref().unwrap().is_file());
        assert!(!job.work.join("resident").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
