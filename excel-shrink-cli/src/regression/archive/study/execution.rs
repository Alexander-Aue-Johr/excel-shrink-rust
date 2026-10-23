use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn process(
    source: &Source,
    index: usize,
    id: &str,
    position: usize,
    mode: JobMode,
    output: &Path,
    records: &mut Vec<Value>,
    corpora: &[Value],
    limits: &ArchiveArgs,
    run: &Value,
    log_level: LogLevel,
) -> Result<()> {
    let key = artifact_cache::sha256(id.as_bytes());
    let work = output.join(".work").join(&key[..20]);
    worker::cleanup_windows(&work)?;
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    let record = record_path(output, id);
    let diagnosis_only = mode == JobMode::Diagnose;
    if !diagnosis_only {
        records[position]["complete"] = json!(false);
    }
    if mode == JobMode::Prepare {
        records[position]["prepared"] = json!(false);
    }
    if !diagnosis_only {
        records[position]["error"] = Value::Null;
    }
    if !diagnosis_only {
        records[position]["status"] = json!(if mode == JobMode::Prepare {
            "shrinking"
        } else {
            "comparing"
        });
    }
    write_record(&record, &records[position])?;
    let job = MemberJob {
        archive: source.path.clone(),
        index,
        identity: id.into(),
        name: records[position]["name"].as_str().unwrap().into(),
        work: work.clone(),
        record: record.clone(),
        report_root: output.into(),
        max_workbook_bytes: limits.max_workbook_bytes,
        mode,
        bundle: Some(bundle_path(output, id)),
        run_manifest: Some(json!({"run_id":run["run_id"],"machine_id":run["machine_id"],
            "machine_manifest":run["machine_manifest"]})),
    };
    let path = work.join("job.json");
    fs::write(&path, serde_json::to_vec_pretty(&job)?)?;
    let phase = if diagnosis_only {
        "diagnosing whitespace"
    } else if mode == JobMode::Prepare {
        "shrinking"
    } else {
        "comparing"
    };
    eprintln!("[study] {} · {phase} · {}", source.name, job.name);
    publish_progress(output, records, corpora, phase);
    let started = Instant::now();
    let outcome = worker::launch(&job, &path, limits, log_level, || {
        let current: Value = serde_json::from_slice(&fs::read(&record)?)?;
        if current != records[position] {
            records[position] = current;
            publish_progress(output, records, corpora, phase);
        }
        Ok(())
    });
    let mut result: Value = serde_json::from_slice(&fs::read(&record)?)?;
    if let Err(error) = outcome {
        if diagnosis_only {
            result["whitespace_diagnosis"]["version"] = json!(whitespace::VERSION);
            result["whitespace_diagnosis"]["status"] = json!("failed");
            result["whitespace_diagnosis"]["error"] = json!(format!("{error:#}"));
        } else {
            result["status"] = json!("skipped");
            result["error"] = json!(format!("{error:#}"));
        }
    }
    worker::cleanup_windows(&work)?;
    retain_visual_evidence(&mut result, &work, output, &key[..20], diagnosis_only);
    relative_images(&mut result, output);
    if result["status"] == "skipped" || result["whitespace_diagnosis"]["status"] == "failed" {
        if let Err(error) = retain_capture_logs(&work, output, &key[..20]) {
            eprintln!("Could not preserve capture diagnostics: {error:#}");
        }
    }
    result[format!("{phase}_elapsed_seconds")] = json!(started.elapsed().as_secs_f64());
    result["complete"] =
        json!(diagnosis_only || mode == JobMode::Compare || result["status"] == "skipped");
    fs::remove_dir_all(&work)?;
    if mode == JobMode::Compare && result["status"] != "skipped" {
        fs::remove_file(bundle_path(output, id))?;
        result["prepared"] = json!(false);
    }
    write_record(&record, &result)?;
    records[position] = result;
    publish_progress(output, records, corpora, phase);
    Ok(())
}

fn retain_visual_evidence(
    result: &mut Value,
    work: &Path,
    output: &Path,
    key: &str,
    diagnosis_only: bool,
) {
    let mut diagnosis = result["whitespace_diagnosis"].take();
    if !diagnosis_only {
        if let Err(error) = worker::retain_evidence(
            result,
            work,
            &output.join("evidence").join(key).join("comparison"),
        ) {
            result["status"] = json!("skipped");
            result["error"] = json!(format!("preserve image evidence: {error:#}"));
            clear_images(result);
        }
    }
    // Supplemental evidence failure must not erase a completed primary check.
    if let Err(error) = worker::retain_evidence(
        &mut diagnosis,
        work,
        &output.join("evidence").join(key).join("whitespace-v1"),
    ) {
        diagnosis["status"] = json!("failed");
        diagnosis["error"] = json!(format!("preserve whitespace evidence: {error:#}"));
        clear_images(&mut diagnosis);
    }
    result["whitespace_diagnosis"] = diagnosis;
}

fn retain_capture_logs(work: &Path, output: &Path, key: &str) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_LOG_BYTES: u64 = 1024 * 1024;
    let logs = output.join("logs");
    fs::create_dir_all(&logs)?;
    for profile in prepared::PROFILES {
        let mut captures = vec![(
            profile.to_owned(),
            work.join("comparisons").join(profile).join("visual/images"),
        )];
        for phase in [
            "without-whitespace-deletion",
            "reproduce-original-comparison",
        ] {
            captures.push((
                format!("{profile}-{phase}"),
                work.join("whitespace")
                    .join(profile)
                    .join(phase)
                    .join("images"),
            ));
        }
        for (label, capture) in captures {
            for name in ["host.stdout.log", "host.stderr.log"] {
                let path = capture.join(name);
                if !path.is_file() {
                    continue;
                }
                let mut source = fs::File::open(path)?;
                let length = source.metadata()?.len();
                source.seek(SeekFrom::Start(length.saturating_sub(MAX_LOG_BYTES)))?;
                let mut tail = Vec::new();
                source.take(MAX_LOG_BYTES).read_to_end(&mut tail)?;
                fs::write(logs.join(format!("{key}-{label}-{name}")), tail)?;
            }
        }
    }
    Ok(())
}

fn relative_images(value: &mut Value, root: &Path) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key.ends_with("_image") {
                    if let Some(path) = value.as_str() {
                        if let Ok(relative) = Path::new(path).strip_prefix(root) {
                            *value = json!(relative.to_string_lossy().replace('\\', "/"));
                        }
                    }
                } else {
                    relative_images(value, root);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                relative_images(item, root);
            }
        }
        _ => {}
    }
}

fn clear_images(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !key.ends_with("_image"));
            for value in map.values_mut() {
                clear_images(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                clear_images(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_supplemental_evidence_preserves_completed_primary_results() -> Result<()> {
        let root = super::super::tests::test_directory("whitespace-evidence-failure")?;
        let primary = json!([{"sheets":[{"original_image":"evidence/old-original.png", "shrunk_image":"evidence/old-shrunk.png"}]}]);
        let mut record = json!({"complete":true,"status":"differences","results":primary,
            "whitespace_diagnosis":{"status":"complete","control":{"original_image":root.join("missing.png")}}});
        retain_visual_evidence(&mut record, &root, &root, "example", true);
        assert_eq!(record["results"], primary);
        assert_eq!(record["status"], "differences");
        assert_eq!(record["complete"], true);
        assert_eq!(record["whitespace_diagnosis"]["status"], "failed");
        assert!(record["whitespace_diagnosis"]["control"]
            .get("original_image")
            .is_none());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn keeps_bounded_failure_logs_without_workbooks_or_images() -> Result<()> {
        let root = super::super::tests::test_directory("failure-logs")?;
        let work = root.join("work");
        let capture = work.join("comparisons/default/visual/images");
        fs::create_dir_all(&capture)?;
        let mut log = vec![b'a'; 2 * 1024 * 1024];
        log.extend_from_slice(b"last failure");
        fs::write(capture.join("host.stdout.log"), log)?;
        fs::write(capture.join("original.xlsx"), b"workbook")?;
        fs::write(capture.join("equal.png"), b"image")?;
        retain_capture_logs(&work, &root, "example")?;
        fs::remove_dir_all(&work)?;
        let kept = fs::read(root.join("logs/example-default-host.stdout.log"))?;
        assert_eq!(kept.len(), 1024 * 1024);
        assert!(kept.ends_with(b"last failure"));
        assert_eq!(fs::read_dir(root.join("logs"))?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
