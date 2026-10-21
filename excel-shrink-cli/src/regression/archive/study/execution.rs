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
    records[position]["complete"] = json!(false);
    if mode == JobMode::Prepare {
        records[position]["prepared"] = json!(false);
    }
    records[position]["error"] = Value::Null;
    records[position]["status"] = json!(if mode == JobMode::Prepare {
        "shrinking"
    } else {
        "comparing"
    });
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
    };
    let path = work.join("job.json");
    fs::write(&path, serde_json::to_vec_pretty(&job)?)?;
    let phase = if mode == JobMode::Prepare {
        "shrinking"
    } else {
        "comparing"
    };
    eprintln!("[study] {} · {phase} · {}", source.name, job.name);
    publish(output, records, corpora, false, false, phase)?;
    let started = Instant::now();
    let outcome = worker::launch(&job, &path, limits, log_level, || {
        let current: Value = serde_json::from_slice(&fs::read(&record)?)?;
        if current != records[position] {
            records[position] = current;
            publish(output, records, corpora, false, false, phase)?;
        }
        Ok(())
    });
    let mut result: Value = serde_json::from_slice(&fs::read(&record)?)?;
    if let Err(error) = outcome {
        result["status"] = json!("skipped");
        result["error"] = json!(format!("{error:#}"));
    }
    worker::cleanup_windows(&work)?;
    if let Err(error) = worker::retain_evidence(
        &mut result,
        &work,
        &output.join("evidence").join(&key[..20]),
    ) {
        result["status"] = json!("skipped");
        result["error"] = json!(format!("preserve image evidence: {error:#}"));
        clear_images(&mut result);
    }
    relative_images(&mut result, output);
    result[format!("{phase}_elapsed_seconds")] = json!(started.elapsed().as_secs_f64());
    result["complete"] = json!(mode == JobMode::Compare || result["status"] == "skipped");
    fs::remove_dir_all(&work)?;
    if mode == JobMode::Compare && result["status"] != "skipped" {
        fs::remove_file(bundle_path(output, id))?;
        result["prepared"] = json!(false);
    }
    write_record(&record, &result)?;
    records[position] = result;
    publish(output, records, corpora, false, false, phase)
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
