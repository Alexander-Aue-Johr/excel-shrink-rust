use super::*;

fn test_directory(name: &str) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("study-{name}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(directory.join("records"))?;
    Ok(directory)
}

#[test]
fn imports_verified_files_without_restarting_them_and_queues_prior_differences() -> Result<()> {
    let output = test_directory("import")?;
    let input = output.join("old-report.json");
    let mut files: Vec<_> = (0..326)
        .map(|i| {
            json!({
                "identity":format!("archive:{i}:file-{i}.xlsx"),
                "name":format!("old archive / file-{i}.xlsx"),
                "complete":true,"status":"unchanged", "results":[{
                    "profile":"maximum-cleaning","status":"visually unchanged",
                    "checks_complete":true,"reduction_ratio":0.9}]
            })
        })
        .collect();
    files.push(json!({"identity":"archive:326:changed.xlsx","name":"changed.xlsx",
        "complete":true,"status":"differences","results":[{"sheets":[{"original_image":"old.png"}]}]}));
    files.push(
        json!({"identity":"archive:327:broken.xlsx","name":"broken.xlsx",
        "complete":true,"status":"skipped","error":"old failure","results":[]}),
    );
    files.push(json!({"identity":"another:0:unrelated.xlsx","complete":true,"status":"unchanged"}));
    fs::write(&input, serde_json::to_vec(&json!({"workbooks":files}))?)?;
    let args = StudyArgs::try_parse_from(["study", "--import-report", input.to_str().unwrap()])?;
    let corpora = vec![json!({"id":"archive","name":"Example corpus"})];
    let mut records = Vec::new();
    import(&args, &output, &corpora, &mut records)?;
    assert_eq!(records.len(), 328);
    assert_eq!(
        records.iter().filter(|r| r["complete"] == true).count(),
        326
    );
    assert!(records[..326]
        .iter()
        .all(|r| r["previously_verified"] == true));
    assert_eq!(records[0]["name"], "file-0.xlsx");
    assert_eq!(records[0]["results"][0]["reduction_ratio"], 0.9);
    assert!(records[0]["results"][0]["shrink_seconds"].is_null());
    assert_eq!(records[326]["status"], "pending");
    assert_eq!(records[326]["previous_difference"], true);
    assert_eq!(records[326]["results"], json!([]));
    assert!(records[327]["error"].is_null());

    // Repeating an import must not replace newer results with the old report.
    records[326]["status"] = json!("unchanged");
    records[326]["complete"] = json!(true);
    import(&args, &output, &corpora, &mut records)?;
    assert_eq!(records.len(), 328);
    assert_eq!(records[326]["status"], "unchanged");

    ledger::write(&output, &records[..326])?;
    let portable = ledger::read(&output.join("verified-unchanged.csv"))?;
    assert_eq!(portable.len(), 326);
    assert_eq!(portable[0]["identity"], records[0]["identity"]);
    assert!(portable[0]["results"][0]["shrink_seconds"].is_null());
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn retry_discards_old_evidence_even_when_preparation_or_unpacking_fails() -> Result<()> {
    let output = test_directory("retry")?;
    let work = output.join("work");
    fs::create_dir_all(&work)?;
    let invalid = output.join("invalid.zip");
    fs::write(&invalid, b"not a ZIP")?;
    let mut job = MemberJob {
        archive: invalid.clone(),
        index: 0,
        identity: "test:0:bad.xlsx".into(),
        name: "bad.xlsx".into(),
        work: work.clone(),
        record: output.join("record.json"),
        report_root: output.clone(),
        max_workbook_bytes: 1024,
        mode: JobMode::Prepare,
        bundle: Some(invalid),
    };
    for mode in [JobMode::Prepare, JobMode::Compare] {
        job.mode = mode;
        let mut record = json!({"results":[{"profile":"maximum-cleaning","sheets":[
            {"original_image":"evidence/old.png","shrunk_image":"evidence/old-shrunk.png"}]}]});
        assert!(run_job(&job, &mut record, LogLevel::Warn).is_err());
        assert_eq!(record["results"], json!([]));
        worker::retain_evidence(&mut record, &work, &output.join("evidence"))?;
    }
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn default_only_success_does_not_skip_the_stronger_comparison() {
    let mut record = json!({"complete":true,"status":"unchanged","results":[{"profile":"default","checks_complete":true,"status":"visually unchanged"}]});
    assert!(!verified_unchanged(&record));
    record["results"][0]["profile"] = json!("maximum-cleaning");
    assert!(verified_unchanged(&record));
    record["results"][0]["checks_complete"] = json!(false);
    assert!(!verified_unchanged(&record));
}
