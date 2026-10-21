//! Shrink serially, then retain the three outputs in disposable ZIP bundles.
use super::*;
use std::io::Write;

pub(super) const PROFILES: [&str; 3] = ["default", "maximum-cleaning", "too-aggressive-cleaning"];

pub(super) fn prepare(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    record["prepared"] = json!(false);
    record["results"] = json!([]);
    let extracted = worker::extract(job)?;
    record["source_bytes"] = json!(fs::metadata(&extracted)?.len());
    record["source_format"] = json!(extracted.extension().and_then(|e| e.to_str()));
    let source = if extracted
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xls"))
    {
        let converted = job.work.join("converted.xlsx");
        let metrics = excel_automation::convert_legacy_workbook(
            &extracted,
            &converted,
            &job.work.join("conversion"),
        )?;
        record["conversion"] = serde_json::to_value(metrics)?;
        converted
    } else {
        extracted
    };
    let baseline = artifact_cache::prepare_baseline(&source)?;
    record["source_sha256"] = json!(baseline.source_sha256);
    record["baseline_sha256"] = json!(baseline.snapshot.manifest.archive_sha256);
    let source = baseline.snapshot.path;
    record["baseline_repaired"] = json!(baseline.workbook_xml_changed);
    let plan = office_plan::build_workbook_visual_plan(&source)
        .context("validate workbook capture plan")?;
    ensure!(
        !plan.sheets.is_empty(),
        "workbook has no visible worksheets to compare"
    );
    let case: RegressionCase =
        serde_json::from_value(json!({"id":"workbook", "input_path":source}))?;
    let mut outputs = vec![("original".to_owned(), source.clone())];
    record["results"] = json!([]);
    for name in PROFILES {
        let predefined = predefined_visual_profiles()
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("unknown profile {name}"))?;
        let report = run_case(
            "study",
            &repo_root(),
            &job.work.join(name),
            None,
            None,
            &predefined.config.cli_args(),
            false,
            false,
            &visual_validation::VisualCheckOptions::default(),
            log_level,
            &case,
        )?;
        ensure!(
            report.failures.is_empty(),
            "{name}: {}",
            report.failures.join("; ")
        );
        artifact_cache::assert_shrink_invariants(
            &source,
            &report.output_path,
            &job.work.join("audit").join(name),
        )?;
        record["results"].as_array_mut().unwrap().push(json!({
            "profile":name,"status":"not compared","checks_complete":false,"failures":[],"sheets":[],
            "original_bytes":report.original_bytes,"shrunk_bytes":report.cleaned_bytes,
            "reduction_ratio":report.reduction_ratio,"shrink_seconds":report.shrink_duration_ms.map(|v|v as f64/1000.0),
            "config_hash":report.config_hash
        }));
        outputs.push((name.into(), report.output_path));
        write_record(&job.record, record)?;
    }
    let bundle = job
        .bundle
        .as_ref()
        .context("missing prepared bundle path")?;
    fs::create_dir_all(bundle.parent().unwrap())?;
    let pending = job.work.join("prepared.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&pending)?);
    // XLSX files are already compressed. Recompressing them adds time, not useful space savings.
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    for (name, path) in outputs {
        zip.start_file(format!("{name}.xlsx"), options)?;
        std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
    }
    zip.start_file("measurements.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&record["results"])?)?;
    zip.finish()?.sync_all()?;
    if bundle.exists() {
        fs::remove_file(bundle)?;
    }
    fs::rename(pending, bundle)?;
    record["prepared"] = json!(true);
    record["status"] = json!("prepared");
    Ok(())
}

fn unpack(bundle: &Path, work: &Path) -> Result<Value> {
    let mut zip = ZipArchive::new(fs::File::open(bundle)?)?;
    for name in std::iter::once("original").chain(PROFILES) {
        let filename = format!("{name}.xlsx");
        let mut entry = zip.by_name(&filename)?;
        std::io::copy(&mut entry, &mut fs::File::create(work.join(filename))?)?;
    }
    let measurements = serde_json::from_reader(zip.by_name("measurements.json")?)?;
    Ok(measurements)
}

pub(super) fn compare(job: &MemberJob, record: &mut Value) -> Result<()> {
    record["results"] = json!([]);
    record["results"] = unpack(
        job.bundle.as_ref().context("missing prepared bundle")?,
        &job.work,
    )?;
    let original = job.work.join("original.xlsx");
    let original_metrics =
        excel_automation::measure_workbook_open(&original, &job.work.join("open-original"))?;
    for name in ["default", "maximum-cleaning"] {
        let metrics = excel_automation::measure_workbook_open(
            &job.work.join(format!("{name}.xlsx")),
            &job.work.join(format!("open-{name}")),
        )?;
        let result = profile_mut(record, name)?;
        add_open_metrics(
            result,
            &serde_json::to_value(&original_metrics)?,
            &serde_json::to_value(metrics)?,
        );
        write_record(&job.record, record)?;
    }
    let mut selected = None;
    for name in ["too-aggressive-cleaning", "maximum-cleaning", "default"] {
        let result = compare_profile(job, name, selected.clone())?;
        let target = profile_mut(record, name)?;
        for (key, value) in result.as_object().unwrap() {
            target[key] = value.clone();
        }
        write_record(&job.record, record)?;
        let Some(changed) = follow_up_sheets(&result) else {
            break;
        };
        selected = Some(changed);
    }
    let profiles = record["results"].as_array().context("missing results")?;
    let technical = profiles.iter().any(|p| p["status"] == "technical error");
    let changed = profiles
        .iter()
        .any(|p| p["failures"].as_array().is_some_and(|a| !a.is_empty()));
    record["status"] = json!(if technical {
        "skipped"
    } else if changed {
        "differences"
    } else {
        "unchanged"
    });
    if technical {
        record["error"] = json!("One or more comparisons could not be completed.");
    }
    Ok(())
}

fn profile_mut<'a>(record: &'a mut Value, name: &str) -> Result<&'a mut Value> {
    record["results"]
        .as_array_mut()
        .context("missing profile measurements")?
        .iter_mut()
        .find(|r| r["profile"] == name)
        .with_context(|| format!("missing {name} measurements"))
}

fn add_open_metrics(result: &mut Value, original: &Value, shrunk: &Value) {
    for (prefix, metrics) in [("original", original), ("shrunk", shrunk)] {
        result[format!("{prefix}_open_seconds")] = metrics["open_seconds"].clone();
        result[format!("{prefix}_memory_bytes")] = metrics["working_set_bytes"].clone();
        result[format!("{prefix}_private_bytes")] = metrics["private_bytes"].clone();
    }
}

fn follow_up_sheets(result: &Value) -> Option<std::collections::BTreeSet<usize>> {
    if result["checks_complete"] != true {
        return None;
    }
    let changed: std::collections::BTreeSet<usize> = result["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["status"] == "changed")
        .filter_map(|s| s["index"].as_u64().map(|n| n as usize))
        .collect();
    (!changed.is_empty()).then_some(changed)
}

fn compare_profile(
    job: &MemberJob,
    name: &str,
    sheets: Option<std::collections::BTreeSet<usize>>,
) -> Result<Value> {
    let original = job.work.join("original.xlsx");
    let shrunk = job.work.join(format!("{name}.xlsx"));
    let directory = job.work.join("comparisons").join(name);
    fs::create_dir_all(&directory)?;
    let predefined = predefined_visual_profiles()
        .iter()
        .find(|p| p.name == name)
        .unwrap();
    let case: RegressionCase =
        serde_json::from_value(json!({"id":"workbook","input_path":original}))?;
    let cli = build_cli(
        &original,
        &directory,
        None,
        &case,
        false,
        &directory.join("analysis.csv"),
        &predefined.config.cli_args(),
    );
    let options = visual_validation::VisualCheckOptions {
        profile: name.into(),
        sheets: sheets.clone(),
        keep_screenshots: false,
        combined_report: Some(job.report_root.join("report.html")),
    };
    let mut result = json!({"profile":name,"status":"technical error","checks_complete":false,"sheets":[],"requested_sheets":sheets});
    let checked = (|| -> Result<Vec<String>> {
        let (values, mut failures) =
            validate_excel_recalculation(&original, &shrunk, &directory, &cli)?;
        // Value differences are recorded but still need images on their affected sheets.
        let visuals = visual_validation::compare_sheet_visuals(
            &original,
            &shrunk,
            &directory.join("visual"),
            &options,
        )?;
        result = serde_json::from_slice(&fs::read(visuals.output_dir.join("visual-result.json"))?)?;
        if visuals.differences_found {
            failures.push("Visual differences found".into());
        }
        result["value_differences"] = json!(
            values.original_vs_shrunk.difference_count + values.real_recalc_differences.len()
        );
        result["stored_value_differences"] = json!(values.original_vs_shrunk.difference_count);
        result["recalculated_value_differences"] = json!(values.real_recalc_differences.len());
        result["checks_complete"] = json!(true);
        result["status"] = json!(if visuals.differences_found {
            "visual differences"
        } else if !failures.is_empty() {
            "cell value differences"
        } else {
            "visually unchanged"
        });
        Ok(failures)
    })();
    result["profile"] = json!(name);
    result["requested_sheets"] = json!(sheets);
    match checked {
        Ok(failures) => result["failures"] = json!(failures),
        Err(error) => {
            result["status"] = json!("technical error");
            result["checks_complete"] = json!(false);
            result["failures"] = json!([format!("{error:#}")]);
        }
    }
    if let Some(items) = result["sheets"].as_array_mut() {
        let checked = items.len();
        let unchanged = items.iter().filter(|s| s["status"] == "same").count();
        if sheets.is_none() {
            items.retain(|s| s["status"] != "same");
        }
        result["checked_sheets"] = json!(checked);
        result["unchanged_sheets"] = json!(unchanged);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn followups_only_include_visually_changed_sheets_of_completed_checks() {
        let mut result = json!({"checks_complete":true,"sheets":[{"index":2,"status":"changed"},{"index":3,"status":"same"}]});
        assert_eq!(
            follow_up_sheets(&result),
            Some(std::collections::BTreeSet::from([2]))
        );
        result["checks_complete"] = json!(false);
        assert!(follow_up_sheets(&result).is_none());
        assert!(follow_up_sheets(
            &json!({"checks_complete":true,"value_differences":2,"sheets":[]})
        )
        .is_none());
    }
}
