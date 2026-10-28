//! Record namespace exclusions reported by the shrinker without opening XML again.
use super::*;

const REASON_PREFIX: &str = "Skipped XLSX with unsupported XML namespaces: ";

pub(super) fn skip_from_batch_report(
    job: &MemberJob,
    record: &mut Value,
    batch_report: &Path,
) -> Result<bool> {
    let bytes = match fs::read(batch_report) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let report: Value = serde_json::from_slice(&bytes)?;
    let files = report["files"].as_array().context("shrink batch report has no files")?;
    ensure!(files.len() == 1, "expected one workbook in shrink batch report");
    let file = &files[0];
    if file["status"] != "skipped" {
        return Ok(false);
    }
    let Some(part) = file["reason"].as_str().and_then(|reason| reason.strip_prefix(REASON_PREFIX)) else {
        return Ok(false);
    };
    let reason = format!(
        "Shrinker skipped unsupported XML namespace or root representation in {part}; visual comparison omitted."
    );
    record["namespace_exclusion"] = json!({
        "code":"UNSUPPORTED_XML_ROOT", "part":part,
        "method":"shrinker-batch-report-v1",
        "scope":"reported by excel-shrink; no additional XML scan"
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
    fn shrinker_namespace_skip_does_not_claim_visual_identity() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-report")?;
        let job = super::super::tests::provenance_job(&root);
        let batch = root.join("batch.json");
        fs::write(&batch, serde_json::to_vec(&json!({"files":[{
            "status":"skipped",
            "reason":"Skipped XLSX with unsupported XML namespaces: xl/worksheets/sheet.xml"
        }]}))?)?;
        let mut record = json!({"status":"unchanged","prepared":true,
            "package_identity":{"status":"equivalent"},"last_execution":{"capture_mode":"resident"},
            "results":[{"profile":"default","shrink_seconds":0.01,"checks_complete":true,
                "status":"visually unchanged","sheets":[{"status":"same"}]}]});
        assert!(skip_from_batch_report(&job, &mut record, &batch)?);
        assert_eq!(record["status"], "skipped");
        assert_eq!(record["namespace_exclusion"]["part"], "xl/worksheets/sheet.xml");
        assert_eq!(record["namespace_exclusion"]["method"], "shrinker-batch-report-v1");
        assert_eq!(record["results"][0]["checks_complete"], false);
        assert!(record["package_identity"].is_null());
        assert_eq!(record["results"][0]["shrink_seconds"], 0.01);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn ordinary_shrink_result_is_not_a_namespace_exclusion() -> Result<()> {
        let root = super::super::super::tests::test_directory("namespace-not-skipped")?;
        let job = super::super::tests::provenance_job(&root);
        let batch = root.join("batch.json");
        fs::write(&batch, serde_json::to_vec(&json!({"files":[{"status":"processed"}]}))?)?;
        let mut record = json!({"status":"preparing","results":[]});
        let before = record.clone();
        assert!(!skip_from_batch_report(&job, &mut record, &batch)?);
        assert_eq!(record, before);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn preparation_uses_the_shrinkers_namespace_result() -> Result<()> {
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
        assert_eq!(record["baseline_sha256"], record["converted_sha256"]);
        assert_eq!(record["namespace_exclusion"]["part"], "xl/worksheets/sheet.xml");
        assert_eq!(record["results"], json!([]));
        assert!(!job.bundle.as_ref().unwrap().exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
