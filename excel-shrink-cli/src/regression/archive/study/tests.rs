use super::*;

#[test]
fn record_index_matches_first_identity_lookup_and_ignores_unidentified_records() {
    let records = vec![json!({"identity":"a"}), json!({}), json!({"identity":"a"}), json!({"identity":"b"})];
    let positions = index_records(&records);
    assert_eq!(positions.get("a"), Some(&0));
    assert_eq!(positions.get("b"), Some(&3));
    assert_eq!(positions.get("missing"), None);
    assert_eq!(positions.len(), 2);
}

#[test]
fn preparation_refresh_requires_a_bounded_explicit_selection() -> Result<()> {
    assert!(StudyArgs::try_parse_from(["study", "--refresh-preparation"]).is_err());
    let args = StudyArgs::try_parse_from([
        "study", "--refresh-preparation", "--workbook-list", "changed.json", "--prepare-only",
    ])?;
    assert!(args.refresh_preparation);
    assert!(args.prepare_only);
    for conflict in ["--compare-only", "--report-only", "--selection-only"] {
        assert!(StudyArgs::try_parse_from([
            "study", "--refresh-preparation", "--workbook-list", "changed.json", conflict,
        ]).is_err());
    }
    assert!(!StudyArgs::try_parse_from(["study"])?.refresh_preparation);
    Ok(())
}

#[test]
fn deferred_full_scan_requires_an_explicit_comparison_run() -> Result<()> {
    assert!(StudyArgs::try_parse_from(["study", "--full-scan-deferred"]).is_err());
    let args = StudyArgs::try_parse_from([
        "study", "--compare-only", "--resident-capture", "--full-scan-deferred",
    ])?;
    assert!(args.full_scan_deferred);
    assert!(has_deferred_full_scan(&json!({
        "capture_deferred": {"default": [1], "maximum-cleaning": []}
    })));
    assert!(!has_deferred_full_scan(&json!({
        "capture_deferred": {"default": [], "maximum-cleaning": []}
    })));
    Ok(())
}

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
            json!({"profile":name,"config_hash":profile.config.config_hash(),
                "shrink_identity":{"workbook_original_archive_reused":false,
                    "workbook_only_cleanup_available":false,"worksheet_xml_changed":true,
                    "cleaned_package_crc_length_sha256":"a".repeat(64),
                    "workbook_xml_cleaned_entries":[],"sheets":{}},
                "cleanup_analysis":{"status":"complete"}})
        })
        .collect();
    let record = json!({"prepared":true,"prepared_by":old_build,"source_fingerprint":"catalog","results":profiles});
    let accepted = vec![old_build.clone()];
    let mut legacy = record.clone();
    legacy["results"][0]
        .as_object_mut()
        .unwrap()
        .remove("shrink_identity");
    assert!(!reusable_preparation(
        &legacy,
        &bundle,
        &old_build,
        "catalog",
        &[],
        None
    ));
    assert!(reusable_preparation(
        &record,
        &bundle,
        &old_build,
        "catalog",
        &[],
        None
    ));
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &[],
        None
    ));
    assert!(reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &accepted,
        None
    ));
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "changed catalog",
        &accepted,
        None
    ));
    assert_eq!(record["prepared_by"], old_build);
    let mut approved = record.clone();
    approved["preparation_accepted_by"] = json!(current_build);
    assert!(reusable_preparation(
        &approved,
        &bundle,
        &current_build,
        "catalog",
        &[],
        None
    ));
    assert!(!reusable_preparation(
        &approved,
        &bundle,
        &"c".repeat(64),
        "catalog",
        &[],
        None
    ));
    assert!(!reusable_preparation(
        &approved,
        &bundle,
        &current_build,
        "changed catalog",
        &[],
        None
    ));
    for field in ["prepared", "prepared_by", "source_fingerprint"] {
        let mut changed = record.clone();
        changed[field] = Value::Null;
        assert!(!reusable_preparation(
            &changed,
            &bundle,
            &current_build,
            "catalog",
            &accepted,
            None
        ));
    }
    let mut changed = record.clone();
    changed["results"][0]["config_hash"] = json!("changed settings");
    assert!(!reusable_preparation(
        &changed,
        &bundle,
        &current_build,
        "catalog",
        &accepted,
        None
    ));
    changed = record.clone();
    changed["results"].as_array_mut().unwrap().pop();
    assert!(!reusable_preparation(
        &changed,
        &bundle,
        &current_build,
        "catalog",
        &accepted,
        None
    ));
    let mut default_only = record.clone();
    default_only["results"]
        .as_array_mut()
        .unwrap()
        .retain(|profile| profile["profile"] == "default");
    assert!(has_required_shrink_evidence(&default_only));
    let targeted = StudyArgs::try_parse_from(["study", "--resident-capture", "--targeted-comparison"])?;
    assert!(has_preparation_for_screen(&default_only, targeted.preparation_profile()),
        "A completed Default-only targeted run must not be requeued for missing unused profiles");
    assert!(reusable_preparation(
        &default_only,
        &bundle,
        &old_build,
        "catalog",
        &[],
        Some("default")
    ));
    assert!(reusable_preparation(
        &record,
        &bundle,
        &old_build,
        "catalog",
        &[],
        Some("default")
    ));
    assert!(!reusable_preparation(
        &default_only,
        &bundle,
        &old_build,
        "catalog",
        &[],
        None
    ));
    assert!(!reusable_preparation(
        &default_only,
        &bundle,
        &old_build,
        "catalog",
        &[],
        Some("maximum-cleaning")
    ));
    let mut default_visual = default_only.clone();
    default_visual["status"] = json!("unchanged");
    default_visual["preparation_disposition"] = json!("needs_visual");
    default_visual["results"][0]["status"] = json!("visually unchanged");
    default_visual["results"][0]["checks_complete"] = json!(true);
    default_visual["results"][0]["failures"] = json!([]);
    assert!(
        !fully_visually_identical(&default_visual),
        "a Default result is not an all-profile certificate"
    );
    assert!(
        !has_preparation_for_screen(&default_visual, None),
        "widening the request requires preparation"
    );
    fs::remove_file(&bundle)?;
    assert!(!reusable_preparation(
        &record,
        &bundle,
        &current_build,
        "catalog",
        &accepted,
        None
    ));
    fs::remove_dir_all(output)?;
    Ok(())
}

#[test]
fn preparation_classification_excludes_nonvisual_workbooks_before_comparison() {
    let make_record = |workbook_only: bool, changed_profile: Option<&str>| {
        let profiles: Vec<_> = prepared::PROFILES.iter().map(|name| {
            let changed = changed_profile == Some(*name);
            json!({"profile":name,"shrink_identity":{
                "workbook_original_archive_reused":!changed,
                "workbook_only_cleanup_available":workbook_only && *name == "default",
                "worksheet_xml_changed":changed,
                "cleaned_package_crc_length_sha256":"a".repeat(64),
                "workbook_xml_cleaned_entries":[],"sheets":{}},
                "cleanup_analysis":{"status":"complete"}})
        }).collect();
        json!({"prepared":true,"status":"prepared","complete":false,"results":profiles})
    };

    let mut unchanged = make_record(false, None);
    assert!(classify_preparation(&mut unchanged));
    assert_eq!(unchanged["preparation_disposition"], "crc_unchanged");
    assert_eq!(unchanged["status"], "crc-identical");
    assert_eq!(unchanged["complete"], true);
    assert_eq!(unchanged["checks_complete"], false);

    let mut workbook_only = make_record(true, None);
    assert!(classify_preparation(&mut workbook_only));
    assert_eq!(workbook_only["preparation_disposition"], "workbook_only");
    assert_eq!(workbook_only["status"], "workbook-only");
    assert_eq!(workbook_only["complete"], true);

    let mut changed = make_record(false, Some("maximum-cleaning"));
    assert!(classify_preparation(&mut changed));
    assert_eq!(changed["preparation_disposition"], "needs_visual");
    assert_eq!(changed["complete"], false);
    assert_eq!(changed["status"], "prepared");

    // A resumed run may already hold a completed comparison. The global
    // preparation barrier must not erase it before a new attempt is admitted.
    changed["complete"] = json!(true);
    changed["status"] = json!("paused-for-later");
    assert!(!classify_preparation(&mut changed));
    assert_eq!(changed["status"], "paused-for-later");
    assert_eq!(changed["complete"], true);

    let mut old_bundle = make_record(false, Some("maximum-cleaning"));
    old_bundle["results"][0].as_object_mut().unwrap().remove("shrink_identity");
    assert!(classify_preparation(&mut old_bundle));
    assert_eq!(old_bundle["preparation_disposition"], "evidence_missing");
    assert_eq!(old_bundle["status"], "skipped");
}

#[test]
fn identical_kpi_requires_every_applicable_profile_to_be_fully_checked() {
    let profiles: Vec<_> = prepared::PROFILES.iter().map(|name| json!({
        "profile":name,"status":"visually unchanged","checks_complete":true,"failures":[],
        "shrink_identity":{"workbook_original_archive_reused":false,
            "workbook_only_cleanup_available":false,"worksheet_xml_changed":true,
            "cleaned_package_crc_length_sha256":"a".repeat(64),
            "workbook_xml_cleaned_entries":[],"sheets":{}},
        "cleanup_analysis":{"status":"complete"}
    })).collect();
    let mut record = json!({"status":"unchanged","preparation_disposition":"needs_visual",
        "results":profiles});
    assert!(fully_visually_identical(&record));
    record["results"][1]["checks_complete"] = json!(false);
    assert!(!fully_visually_identical(&record));
    record["results"][1]["checks_complete"] = json!(true);
    record["capture_deferred"] = json!({"default":[7]});
    assert!(!fully_visually_identical(&record));
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
        profile: None,
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

#[test]
fn aggressive_first_requires_resident_and_keeps_normal_default() -> Result<()> {
    assert!(!StudyArgs::try_parse_from(["study"])?.aggressive_first);
    assert!(StudyArgs::try_parse_from(["study", "--aggressive-first"]).is_err());
    let args = StudyArgs::try_parse_from(["study", "--aggressive-first", "--resident-capture", "--control-room"])?;
    assert!(args.aggressive_first);
    assert!(StudyArgs::try_parse_from(["study", "--aggressive-first", "--resident-capture", "--prepare-only"]).is_err());
    Ok(())
}

#[test]
fn preparation_hash_profile_selects_one_resident_screening_mode() -> Result<()> {
    for profile in prepared::PROFILES {
        let args = StudyArgs::try_parse_from([
            "study", "--resident-capture", "--preparation-hash-profile", profile,
        ])?;
        assert_eq!(args.preparation_hash_profile.as_deref(), Some(profile));
        let record = json!({"comparison_pass":strategy::HASH_SCREEN,
            "comparison_hash_profile":profile});
        assert_eq!(strategy::profiles(&record), &[profile]);
    }
    assert!(StudyArgs::try_parse_from([
        "study", "--preparation-hash-profile", "default",
    ]).is_err());
    assert!(StudyArgs::try_parse_from([
        "study", "--resident-capture", "--aggressive-first",
        "--preparation-hash-profile", "default",
    ]).is_err());
    Ok(())
}
#[test]
fn namespace_exclusion_stays_terminal_without_claiming_visual_identity() {
    let mut record = json!({"status":"skipped","prepared":false,
        "namespace_exclusion":{"code":"UNSUPPORTED_XML_ROOT","part":"xl/workbook.xml"},
        "error":"Unsupported XML namespace in xl/workbook.xml; visual comparison omitted."});
    assert!(classify_preparation(&mut record));
    assert_eq!(record["preparation_disposition"], "unsupported_namespace");
    assert_eq!(record["complete"], true);
    assert!(!fully_visually_identical(&record));
    let classified = record.clone();
    assert!(!classify_preparation(&mut record));
    assert_eq!(record, classified);
    assert!(record["error"].as_str().unwrap().contains("xl/workbook.xml"));
}
