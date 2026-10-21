//! Shrink serially, then retain the three outputs in disposable ZIP bundles.
use super::*;
use std::io::Write;

pub(super) const PROFILES: [&str; 3] = ["default", "maximum-cleaning", "too-aggressive-cleaning"];

pub(super) fn prepare(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    record["prepared"] = json!(false);
    record["results"] = json!([]);
    let extracted = worker::extract(job)?;
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
