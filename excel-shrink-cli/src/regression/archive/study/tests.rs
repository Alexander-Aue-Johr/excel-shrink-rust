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
