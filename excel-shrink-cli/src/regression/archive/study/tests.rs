use super::*;

#[test]
fn skipped_retries_default_on_with_compatible_flag_and_explicit_opt_out() -> Result<()> {
    let defaults = StudyArgs::try_parse_from(["study"])?;
    assert!(defaults.retry_skipped);
    assert!(defaults.should_retry_skipped());
    let explicit = StudyArgs::try_parse_from(["study", "--retry-skipped"])?;
    assert!(explicit.should_retry_skipped());
    let disabled = StudyArgs::try_parse_from(["study", "--no-retry-skipped"])?;
    assert!(!disabled.should_retry_skipped());
    for flags in [
        ["--retry-skipped", "--no-retry-skipped"],
        ["--no-retry-skipped", "--retry-skipped"],
    ] {
        assert!(StudyArgs::try_parse_from(["study", flags[0], flags[1]]).is_err());
    }
    Ok(())
}

#[test]
fn manual_control_requires_resident_and_excludes_automatic_admission() -> Result<()> {
    let args = StudyArgs::try_parse_from([
        "study", "--resident-capture", "--control-room", "--comparison-workers", "128",
        "--control-room-port", "8781",
    ])?;
    assert!(args.control_room);
    assert_eq!(args.comparison_workers, 128);
    assert_eq!(args.control_room_port, 8781);
    assert!(StudyArgs::try_parse_from(["study", "--control-room"]).is_err());
    assert!(StudyArgs::try_parse_from(["study", "--resident-capture", "--control-room", "--adaptive-concurrency"]).is_err());
    Ok(())
}

#[test]
fn portable_result_commands_keep_machine_alias_and_repeatable_imports() -> Result<()> {
    let args = StudyArgs::try_parse_from([
        "study", "--report-only", "--machine-label", "Enron laptop 1",
        "--import-results", "one.zip", "--import-results", "two.zip",
        "--export-results", "combined.zip",
    ])?;
    assert_eq!(args.machine_label.as_deref(), Some("Enron laptop 1"));
    assert_eq!(args.import_results, vec![PathBuf::from("one.zip"), PathBuf::from("two.zip")]);
    assert_eq!(args.export_results, Some(PathBuf::from("combined.zip")));
    assert!(args.report_only);
    Ok(())
}

#[test]
fn a_blocked_dashboard_preserves_records_and_recovers_at_the_next_checkpoint() -> Result<()> {
    let output = test_directory("blocked-dashboard")?;
    fs::create_dir_all(output.join("records"))?;
    let record = json!({
        "identity":"archive:0:example.xlsx", "name":"example.xlsx",
        "corpus_id":"archive", "corpus_name":"Example",
        "complete":false, "status":"shrinking", "error":null, "results":[]
    });
    let path = record_path(&output, record["identity"].as_str().unwrap());
    write_record(&path, &record)?;
    let saved = fs::read(&path)?;
    let records = vec![record.clone()];
    let corpora = vec![json!({"id":"archive", "name":"Example", "total_workbooks":1})];
    let dashboard = output.join("report.html");
    // A directory cannot be replaced by a report file, on any supported OS.
    fs::create_dir(&dashboard)?;

    publish_progress(&output, &records, &corpora, "shrinking");
    assert!(dashboard.is_dir());
    assert_eq!(fs::read(&path)?, saved);
    assert_eq!(records[0], record);
    // Explicit report generation and final publication must still report failure.
    assert!(publish(&output, &records, &corpora, false, false, "shrinking").is_err());

    fs::remove_dir(&dashboard)?;
    // The same checkpoint can be retried without rerunning or changing a workbook.
    publish_progress(&output, &records, &corpora, "shrinking");
    assert!(fs::read_to_string(&dashboard)?.contains("example.xlsx"));
    let report: Value = serde_json::from_slice(&fs::read(output.join("diagnosis.json"))?)?;
    let mut annotated = record.clone();
    diagnosis_report::ensure_whitespace_diagnosis(&mut annotated);
    assert_eq!(report["workbooks"], json!([annotated]));
    assert_eq!(report["workbooks"][0]["whitespace_diagnosis"]["status"], "not_investigated");
    assert_eq!(report["phase"], "shrinking");
    assert_eq!(fs::read(&path)?, saved);
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn reviewed_build_reuse_still_requires_matching_source_and_profiles() -> Result<()> {
    let output = test_directory("reuse")?;
    let bundle = output.join("prepared.zip");
    fs::write(
        &bundle,
        b"bundle presence; contents are checked when unpacked",
    )?;
    let old_build = "a".repeat(64);
    let current_build = "b".repeat(64);
    let profiles: Vec<_> = prepared::PROFILES
        .iter()
        .map(|name| {
            let profile = predefined_visual_profiles()
                .iter()
                .find(|p| p.name == *name)
                .unwrap();
            json!({"profile":name,"config_hash":profile.config.config_hash()})
        })
        .collect();
    let record = json!({"prepared":true,"prepared_by":old_build,"source_fingerprint":"catalog","results":profiles});
    let accepted = vec![old_build.clone()];
    assert!(reusable_preparation(
        &record,
        &bundle,
        &old_build,
        "catalog",
        &[]
    ));
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &[]
    ));
    assert!(reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &accepted
    ));
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "changed catalog",
        &accepted
    ));
    assert_eq!(record["prepared_by"], old_build);
    let mut approved = record.clone();
    approved["preparation_accepted_by"] = json!(current_build);
    assert!(reusable_preparation(
        &approved,
        &bundle,
        &current_build,
        "catalog",
        &[]
    ));
    assert!(!reusable_preparation(
        &approved,
        &bundle,
        &"c".repeat(64),
        "catalog",
        &[]
    ));
    assert!(!reusable_preparation(
        &approved,
        &bundle,
        &current_build,
        "changed catalog",
        &[]
    ));
    for field in ["prepared", "prepared_by", "source_fingerprint"] {
        let mut changed = record.clone();
        changed[field] = Value::Null;
        assert!(!reusable_preparation(
            &changed,
            &bundle,
            &current_build,
            "catalog",
            &accepted
        ));
    }
    let mut changed = record.clone();
    changed["results"][0]["config_hash"] = json!("changed settings");
    assert!(!reusable_preparation(
        &changed,
        &bundle,
        &current_build,
        "catalog",
        &accepted
    ));
    changed = record.clone();
    changed["results"].as_array_mut().unwrap().pop();
    assert!(!reusable_preparation(
        &changed,
        &bundle,
        &current_build,
        "catalog",
        &accepted
    ));
    fs::remove_file(&bundle)?;
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &accepted
    ));
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn reuse_requires_full_build_hashes() {
    assert!(StudyArgs::try_parse_from(["study", "--reuse-prepared-from", "abc"]).is_err());
    assert!(preparation_build_hash(&"z".repeat(64)).is_err());
    assert_eq!(
        preparation_build_hash(&"A".repeat(64)).unwrap(),
        "a".repeat(64)
    );
}

pub(super) fn test_directory(name: &str) -> Result<PathBuf> {
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
        "complete":true,"status":"skipped","error":"old failure","results":[{
            "profile":"maximum-cleaning","failures":["Workbook startup timed out"],
            "sheets":[{"original_image":"old.png"}]
        }]}),
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
        327
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
    assert_eq!(records[327]["status"], "skipped");
    assert_eq!(
        records[327]["error"],
        "old failure\nmaximum-cleaning: Workbook startup timed out"
    );
    assert_eq!(records[327]["previously_skipped"], true);
    assert_eq!(records[327]["results"], json!([]));

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
        run_manifest: None,
        resident_capture: false,
        hide_excel: false,
        control_room: None,
            cleanup_barrier: None,
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

#[test]
fn stable_ids_are_explicit_and_legacy_ids_are_unchanged() -> Result<()> {
    let mut corpus = Corpus {
        name: "Example".into(),
        url: "C:/one/archive.zip".into(),
        id: None,
        total_workbooks: None,
    };
    assert_eq!(
        corpus_id(&corpus)?,
        artifact_cache::sha256(corpus.url.as_bytes())
    );
    corpus.id = Some("A".repeat(64));
    let first = corpus_id(&corpus)?;
    corpus.url = "D:/other/archive.zip".into();
    assert_eq!(corpus_id(&corpus)?, first);
    assert_eq!(first, "a".repeat(64));
    corpus.id = Some("invalid".into());
    assert!(corpus_id(&corpus).is_err());
    Ok(())
}

#[test]
fn comparison_workers_default_by_capture_mode_and_allow_explicit_overrides() -> Result<()> {
    let defaults = StudyArgs::try_parse_from(["study"])?;
    assert_eq!(defaults.comparison_workers, 1);
    assert!(!defaults.resident_capture);
    assert!(!defaults.adaptive_concurrency);
    assert!(!defaults.hide_excel);
    let resident = StudyArgs::try_parse_from(["study", "--resident-capture"])?;
    assert_eq!(resident.comparison_workers, 8);
    let adaptive =
        StudyArgs::try_parse_from(["study", "--resident-capture", "--adaptive-concurrency"])?;
    assert_eq!(adaptive.comparison_workers, 8);
    let hidden = StudyArgs::try_parse_from(["study", "--resident-capture", "--hide-excel"])?;
    assert!(hidden.hide_excel);
    assert_eq!(hidden.comparison_workers, 8);
    for count in [1, 3, 24, 256, usize::MAX] {
        let value = count.to_string();
        for args in [
            [
                "study",
                "--resident-capture",
                "--comparison-workers",
                &value,
            ],
            [
                "study",
                "--comparison-workers",
                &value,
                "--resident-capture",
            ],
        ] {
            let parallel = StudyArgs::try_parse_from(args)?;
            assert_eq!(parallel.comparison_workers, count);
            assert!(parallel.resident_capture);
        }
    }
    let paused = StudyArgs::try_parse_from([
        "study",
        "--resident-capture",
        "--control-room",
        "--comparison-workers",
        "0",
    ])?;
    assert_eq!(paused.comparison_workers, 0);
    let unsupported_pause = StudyArgs::try_parse_from(["study", "--comparison-workers", "0"])?;
    assert!(
        format!("{:#}", run(unsupported_pause, LogLevel::Warn).unwrap_err())
            .contains("zero comparison groups require --control-room")
    );
    for invalid in ["-1", "1.5", "18446744073709551616"] {
        assert!(StudyArgs::try_parse_from(["study", "--comparison-workers", invalid]).is_err());
    }
    let unsupported = StudyArgs::try_parse_from(["study", "--comparison-workers", "2"])?;
    assert!(
        format!("{:#}", run(unsupported, LogLevel::Warn).unwrap_err())
            .contains("parallel workbook comparisons require --resident-capture")
    );
    Ok(())
}

#[test]
fn processing_order_and_adaptive_ceiling_are_explicit_cli_options() -> Result<()> {
    let defaults = StudyArgs::try_parse_from(["study"])?;
    assert_eq!(defaults.order, selection::ProcessingOrder::Priority);
    let args = StudyArgs::try_parse_from([
        "study",
        "--order",
        "size-desc",
        "--resident-capture",
        "--adaptive-concurrency",
        "--comparison-workers",
        "24",
    ])?;
    assert_eq!(args.order, selection::ProcessingOrder::SizeDesc);
    assert!(args.adaptive_concurrency);
    assert_eq!(args.comparison_workers, 24);
    let ascending = StudyArgs::try_parse_from(["study", "--order", "size-asc"])?;
    assert_eq!(ascending.order, selection::ProcessingOrder::SizeAsc);
    assert!(StudyArgs::try_parse_from(["study", "--order", "random"]).is_err());
    assert!(StudyArgs::try_parse_from(["study", "--adaptive-concurrency"]).is_err());
    Ok(())
}
