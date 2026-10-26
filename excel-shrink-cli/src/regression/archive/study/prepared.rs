//! Shrink serially, then retain the source and three outputs in reusable ZIP bundles.
use super::*;
use std::io::Write;
mod package_audit;
mod namespace_exclusion;
mod resident;
mod visual_reuse;

pub(super) const PROFILES: [&str; 3] = ["default", "maximum-cleaning", "too-aggressive-cleaning"];

// A new attempt can fail before its worker unpacks the preparation bundle.
// Clear old startup claims at admission, not only once the worker begins.
pub(super) fn clear_startup_measurements(record: &mut Value) {
    for profile in record["results"].as_array_mut().into_iter().flatten() {
        let Some(fields) = profile.as_object_mut() else { continue };
        for field in ["native_startup", "open_measurement_provenance",
            "original_open_seconds", "shrunk_open_seconds", "original_memory_bytes",
            "shrunk_memory_bytes", "original_private_bytes", "shrunk_private_bytes"] {
            fields.remove(field);
        }
    }
}

pub(super) fn prepare(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    provenance::retire(record, provenance::Stage::Preparation);
    provenance::retire(record, provenance::Stage::Comparison);
    provenance::retire(record, provenance::Stage::WhitespaceDiagnosis);
    record["whitespace_diagnosis"] = json!({"status":"not_investigated"});
    record.as_object_mut().unwrap().remove("package_identity");
    record.as_object_mut().unwrap().remove("namespace_exclusion");
    record["prepared"] = json!(false);
    record["results"] = json!([]);
    if let Some(run) = &job.run_manifest {
        provenance::stamp(record, run, provenance::Stage::Preparation);
    }
    // Persist the reset before extraction/conversion can fail or the worker can
    // be killed, otherwise its old comparison machine survives in the journal.
    write_record(&job.record, record)?;
    work_state::event(
        "extracting",
        "extraction_started",
        "Extracting source workbook",
        false,
        None,
        json!({}),
    );
    let extracted = worker::extract(job)?;
    work_state::event(
        "extracting",
        "extraction_complete",
        "Source workbook extracted",
        true,
        None,
        json!({}),
    );
    record["source_bytes"] = json!(fs::metadata(&extracted)?.len());
    record["source_format"] = json!(extracted.extension().and_then(|e| e.to_str()));
    record["source_sha256"] = json!(artifact_cache::hash_file(&extracted)?);
    let source = if extracted
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xls"))
    {
        let converted = job.work.join("converted.xlsx");
        work_state::event(
            "converting",
            "conversion_started",
            "Converting legacy workbook through Excel",
            false,
            Some("Excel legacy conversion"),
            json!({}),
        );
        let metrics = excel_automation::convert_legacy_workbook(
            &extracted,
            &converted,
            &job.work.join("conversion"),
        )?;
        record["conversion"] = serde_json::to_value(metrics)?;
        work_state::event(
            "converting",
            "conversion_complete",
            "Legacy workbook converted",
            true,
            None,
            json!({}),
        );
        converted
    } else {
        extracted
    };
    if namespace_exclusion::skip(job, record, &source)? {
        return Ok(());
    }
    record["converted_sha256"] = json!(artifact_cache::hash_file(&source)?);
    work_state::event(
        "validating",
        "baseline_started",
        "Validating source package and capture plan",
        false,
        None,
        json!({}),
    );
    let baseline = artifact_cache::prepare_baseline(&source)?;
    record["baseline_sha256"] = json!(baseline.snapshot.manifest.archive_sha256);
    let converted = source;
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
        work_state::event(
            "shrinking",
            "profile_started",
            "Shrinking and auditing profile",
            false,
            None,
            json!({"profile":name}),
        );
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
        work_state::event(
            "shrinking",
            "profile_complete",
            "Profile shrunk and audited",
            true,
            None,
            json!({"profile":name}),
        );
    }
    let bundle = job
        .bundle
        .as_ref()
        .context("missing prepared bundle path")?;
    fs::create_dir_all(bundle.parent().unwrap())?;
    work_state::event(
        "saving",
        "bundle_started",
        "Saving reusable preparation bundle",
        false,
        None,
        json!({}),
    );
    let pending = job.work.join("prepared.zip");
    save_bundle(&pending, bundle, &outputs, &converted, record)?;
    record["prepared"] = json!(true);
    record["status"] = json!("prepared");
    work_state::event(
        "saving",
        "bundle_complete",
        "Preparation bundle saved",
        true,
        None,
        json!({}),
    );
    Ok(())
}

fn save_bundle(
    pending: &Path,
    bundle: &Path,
    outputs: &[(String, PathBuf)],
    converted: &Path,
    record: &Value,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(pending)?);
    // XLSX files are already compressed. Recompressing them adds time, not useful space savings.
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut variants = Vec::new();
    for (name, path) in outputs {
        let sha256 = artifact_cache::hash_file(path)?;
        if name == "original" {
            ensure!(sha256 == record["baseline_sha256"],
                "baseline workbook changed before preparation was saved");
        }
        zip.start_file(format!("{name}.xlsx"), options)?;
        std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
        if name != "original" {
            let measurements = record["results"]
                .as_array()
                .context("missing measurements")?;
            let profile = measurements
                .iter()
                .find(|item| item["profile"] == *name)
                .with_context(|| format!("missing {name} measurements"))?;
            variants.push(json!({"profile":name,"member":format!("{name}.xlsx"),
                "sha256":sha256,"config_hash":profile["config_hash"]}));
        }
    }
    ensure!(
        artifact_cache::hash_file(converted)? == record["converted_sha256"],
        "converted workbook changed before preparation was saved"
    );
    let converted_member = if record["converted_sha256"] == record["baseline_sha256"] {
        "original.xlsx"
    } else {
        zip.start_file("converted.xlsx", options)?;
        std::io::copy(&mut fs::File::open(converted)?, &mut zip)?;
        "converted.xlsx"
    };
    let metadata = json!({"version":1,"identity":record["identity"],"name":record["name"],
        "source_sha256":record["source_sha256"],"source_format":record["source_format"],
        "source_fingerprint":record["source_fingerprint"],"prepared_by":record["prepared_by"],
        "converted_sha256":record["converted_sha256"],"converted_member":converted_member,
        "conversion":record["conversion"],"baseline_sha256":record["baseline_sha256"],
        "baseline_repaired":record["baseline_repaired"],"variants":variants,
        "provenance":record["provenance"]["preparation"]});
    zip.start_file("preparation.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&metadata)?)?;
    zip.start_file("measurements.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&record["results"])?)?;
    zip.finish()?.sync_all()?;
    if bundle.exists() {
        fs::remove_file(bundle)?;
    }
    fs::rename(pending, bundle)?;
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
    provenance::retire(record, provenance::Stage::Comparison);
    provenance::retire(record, provenance::Stage::WhitespaceDiagnosis);
    record["whitespace_diagnosis"] = json!({"status":"not_investigated"});
    record.as_object_mut().unwrap().remove("package_identity");
    record.as_object_mut().unwrap().remove("namespace_exclusion");
    record["results"] = json!([]);
    write_record(&job.record, record)?;
    work_state::event(
        "extracting",
        "bundle_opening",
        "Unpacking prepared workbook variants",
        false,
        None,
        json!({}),
    );
    record["results"] = unpack(
        job.bundle.as_ref().context("missing prepared bundle")?,
        &job.work,
    )?;
    if let Some(run) = &job.run_manifest {
        provenance::stamp(record, run, provenance::Stage::Comparison);
    }
    write_record(&job.record, record)?;
    let original = job.work.join("original.xlsx");
    if namespace_exclusion::skip(job, record, &original)? {
        return Ok(());
    }
    work_state::event(
        "validating",
        "package_audit",
        "Checking package equivalence",
        false,
        None,
        json!({}),
    );
    if finish_unchanged_package(job, record, &original)? {
        return Ok(());
    }
    // Resident clients measure native launch through add-in readiness themselves.
    // Do not open these workbooks again through COM just for dashboard metrics.
    if job.resident_capture {
        return resident::compare(job, record);
    }
    // The classic fallback has no native readiness handshake. Leave its startup
    // metrics unavailable rather than launching extra measurement-only Excels.
    let mut selected = None;
    let mut visual_comparisons = visual_reuse::VisualComparisons::default();
    for name in ["too-aggressive-cleaning", "maximum-cleaning", "default"] {
        let result = compare_profile(job, name, selected.clone(), &mut visual_comparisons)?;
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
    } else if changed {
        super::whitespace::run(job, record, LogLevel::Warn);
    }
    Ok(())
}

// An unchanged package needs no Excel run. Check every profile and every ZIP
// member before taking this path; equal worksheet XML alone is not sufficient.
fn finish_unchanged_package(job: &MemberJob, record: &mut Value, original: &Path) -> Result<bool> {
    let variants: Vec<_> = PROFILES
        .iter()
        .map(|name| (*name, job.work.join(format!("{name}.xlsx"))))
        .collect();
    let Some(mut certificate) = package_audit::compare(original, &variants)? else {
        return Ok(false);
    };
    ensure!(
        record["baseline_sha256"].is_string()
            && record["baseline_sha256"] == certificate["original"]["archive_sha256"],
        "prepared original differs from its recorded baseline fingerprint"
    );
    certificate["baseline_repaired"] = record["baseline_repaired"].clone();
    certificate["source_sha256"] = record["source_sha256"].clone();
    for name in PROFILES {
        let profile = profile_mut(record, name)?;
        let fields = profile.as_object_mut().unwrap();
        // Never carry old visual claims or opening measurements into a package
        // audit. Preserve only the actual preparation measurements and identity.
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
        profile["status"] = json!("package unchanged");
        profile["checks_complete"] = json!(false);
        profile["package_checks_complete"] = json!(true);
        profile["capture_method"] = json!("package member SHA-256");
        profile["image_comparison"] = json!("not run; package contents equivalent");
        profile["failures"] = json!([]);
        profile["sheets"] = json!([]);
    }
    record["package_identity"] = certificate;
    record["status"] = json!("unchanged");
    record["error"] = Value::Null;
    record["capture_mode"] = json!("package-audit");
    if record["last_execution"].is_object() {
        record["last_execution"]["capture_mode"] = json!("package-audit");
    }
    record.as_object_mut().unwrap().remove("resident_capture");
    record["whitespace_diagnosis"] = json!({
        "status":"not_applicable", "reason":"all cleaning profiles have equivalent package contents"
    });
    if record["comparison_fallback"].is_object() {
        // A previous interrupted capture may already have scheduled a fallback.
        // Keep its evidence, but do not present that attempt as still running.
        record["comparison_fallback"]["status"] = json!("superseded_by_package_audit");
    }
    eprintln!(
        "[study] {}: all three package contents equivalent; Excel comparison omitted",
        job.name
    );
    write_record(&job.record, record)?;
    Ok(true)
}

fn profile_mut<'a>(record: &'a mut Value, name: &str) -> Result<&'a mut Value> {
    record["results"]
        .as_array_mut()
        .context("missing profile measurements")?
        .iter_mut()
        .find(|r| r["profile"] == name)
        .with_context(|| format!("missing {name} measurements"))
}

#[cfg(test)]
fn add_open_metrics(result: &mut Value, original: &Value, shrunk: &Value) {
    for (prefix, metrics) in [("original", original), ("shrunk", shrunk)] {
        result[format!("{prefix}_open_seconds")] = metrics["open_seconds"].clone();
        result[format!("{prefix}_memory_bytes")] = metrics["working_set_bytes"].clone();
        result[format!("{prefix}_private_bytes")] = metrics["private_bytes"].clone();
    }
}

#[cfg(test)]
fn record_open_metrics(
    job: &MemberJob,
    record: &mut Value,
    name: &str,
    original: &Value,
    shrunk: &Value,
) -> Result<()> {
    add_open_metrics(profile_mut(record, name)?, original, shrunk);
    if let Some(run) = &job.run_manifest {
        provenance::stamp(record, run, provenance::Stage::Comparison);
    }
    // Pair attribution and raw metrics share one durable checkpoint. A later
    // timeout must not leave these measurements assigned to a previous machine.
    write_record(&job.record, record)
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
    visual_comparisons: &mut visual_reuse::VisualComparisons,
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
        work_state::event("validating_values", "classic_recalculation_started",
            "Checking cell values before classic capture", false, Some("Excel recalculation"),
            json!({"profile":name}));
        let (values, mut failures) =
            validate_excel_recalculation(&original, &shrunk, &directory, &cli)?;
        work_state::event("comparing", "classic_recalculation_complete",
            "Cell values checked; preparing classic comparison", true, None,
            json!({"profile":name}));
        // Value differences are recorded but still need images on their affected sheets.
        // Recalculation and open measurements remain independent per profile.
        let original_sha = artifact_cache::hash_file(&original)?;
        let shrunk_sha = artifact_cache::hash_file(&shrunk)?;
        let cached = visual_comparisons
            .find(&original_sha, &shrunk_sha, &sheets, name)
            .filter(|_| {
                [&original, &shrunk].iter().all(|path| {
                    artifact_cache::manifest(path)
                        .is_ok_and(|manifest| reusable_visual_manifest(&manifest))
                })
            });
        let reused = cached.is_some();
        let mut visual = if let Some(report) = cached {
            eprintln!(
                "[study] {name}: reusing byte-identical visual comparison from {}",
                report["comparison_reused_from"].as_str().unwrap()
            );
            report
        } else {
            work_state::event("waiting_client", "classic_capture_started",
                "Classic capture is running in a separate Excel pair", false,
                Some("classic capture process"), json!({"profile":name}));
            let captured = visual_validation::compare_sheet_visuals(
                &original,
                &shrunk,
                &directory.join("visual"),
                &options,
            )?;
            serde_json::from_slice(&fs::read(captured.output_dir.join("visual-result.json"))?)?
        };
        visual["comparison_input_sha256"] = json!({"original":original_sha,"shrunk":shrunk_sha});
        let differences_found = visual["sheets"]
            .as_array()
            .is_some_and(|items| items.iter().any(|sheet| sheet["status"] == "changed"));
        result = visual.clone();
        if differences_found {
            failures.push("Visual differences found".into());
        }
        result["value_differences"] = json!(
            values.original_vs_shrunk.difference_count + values.real_recalc_differences.len()
        );
        result["stored_value_differences"] = json!(values.original_vs_shrunk.difference_count);
        result["recalculated_value_differences"] = json!(values.real_recalc_differences.len());
        result["checks_complete"] = json!(true);
        result["status"] = json!(if differences_found {
            "visual differences"
        } else if !failures.is_empty() {
            "cell value differences"
        } else {
            "visually unchanged"
        });
        if !reused {
            // Keep unchanged sheets in the in-memory entry, before report compaction.
            visual_comparisons.remember(
                &result,
                visual,
                original_sha,
                shrunk_sha,
                sheets.is_none(),
            );
        }
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
    work_state::event("processing_result", "classic_comparison_complete",
        "Classic profile comparison returned", true, None,
        json!({"profile":name,"status":result["status"],"checks_complete":result["checks_complete"]}));
    Ok(result)
}

pub(super) fn reusable_visual_manifest(manifest: &artifact_cache::PackageManifest) -> bool {
    // Byte equality is insufficient for volatile functions or refresh-on-open.
    // Unknown calculation inputs also retain their independent render.
    !manifest.parts.is_empty()
        && manifest
            .parts
            .values()
            .all(|part| !part.visual_runtime && part.recalc_cacheable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_attempt_removes_old_startup_claims_but_keeps_preparation_measurements() {
        let mut record = json!({"results":[{"profile":"default","original_bytes":1000,
            "shrunk_bytes":500,"shrink_seconds":1.0,"original_open_seconds":20.0,
            "shrunk_open_seconds":10.0,"original_memory_bytes":200,"shrunk_memory_bytes":100,
            "original_private_bytes":300,"shrunk_private_bytes":150,
            "open_measurement_provenance":{"run_id":"old"},
            "native_startup":{"method":"launch-to-addin-ready-v1","original":{"seconds":3.0}}}]});
        clear_startup_measurements(&mut record);
        assert_eq!(record["results"][0],json!({"profile":"default","original_bytes":1000,
            "shrunk_bytes":500,"shrink_seconds":1.0}));
    }

    #[test]
    fn saved_bundle_keeps_conversion_and_profile_identity_after_work_cleanup() -> Result<()> {
        for repaired in [false, true] {
            let root = super::super::tests::test_directory("reusable-bundle")?;
            let (job, mut record) = package_job(&root)?;
            let converted = job.work.join("converted.xlsx");
            package_fixture(&converted, b"<styleSheet/>", !repaired)?;
            record["identity"] = json!("archive:0:example.xls");
            record["source_sha256"] = json!("source-bytes-hash");
            record["converted_sha256"] = json!(artifact_cache::hash_file(&converted)?);
            record["baseline_repaired"] = json!(repaired);
            record["prepared_by"] = json!("producer-binary-hash");
            record["provenance"]["preparation"] = json!({"machine_id":"conversion-pc"});
            let outputs: Vec<_> = std::iter::once("original")
                .chain(PROFILES)
                .map(|name| (name.to_owned(), job.work.join(format!("{name}.xlsx"))))
                .collect();
            let bundle = root.join("prepared.zip");
            save_bundle(
                &job.work.join("pending.zip"),
                &bundle,
                &outputs,
                &converted,
                &record,
            )?;
            fs::remove_dir_all(&job.work)?;

            let mut zip = ZipArchive::new(fs::File::open(&bundle)?)?;
            let metadata: Value = serde_json::from_reader(zip.by_name("preparation.json")?)?;
            assert_eq!(metadata["source_sha256"], "source-bytes-hash");
            assert_eq!(metadata["converted_sha256"], record["converted_sha256"]);
            assert_eq!(metadata["baseline_sha256"], record["baseline_sha256"]);
            assert_eq!(metadata["prepared_by"], "producer-binary-hash");
            assert_eq!(metadata["provenance"]["machine_id"], "conversion-pc");
            let converted_member = if repaired {
                "converted.xlsx"
            } else {
                "original.xlsx"
            };
            assert_eq!(metadata["converted_member"], converted_member);
            assert_eq!(zip.len(), if repaired { 7 } else { 6 });
            for profile in metadata["variants"].as_array().unwrap() {
                let mut member = zip.by_name(profile["member"].as_str().unwrap())?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut member, &mut bytes)?;
                assert_eq!(profile["sha256"], artifact_cache::sha256(&bytes));
                assert_eq!(profile["config_hash"], "example");
            }
            drop(zip);
            let reopened = root.join("reopened");
            fs::create_dir(&reopened)?;
            assert_eq!(unpack(&bundle, &reopened)?, record["results"]);
            assert_eq!(
                artifact_cache::hash_file(&reopened.join("original.xlsx"))?,
                record["baseline_sha256"]
            );
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }

    fn package_fixture(path: &Path, styles: &[u8], stored: bool) -> Result<()> {
        let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
        let compression = if stored {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let options = zip::write::FileOptions::default().compression_method(compression);
        for (name, bytes) in [
            ("[Content_Types].xml", b"<Types/>".as_slice()),
            ("xl/workbook.xml", b"<workbook><sheets><sheet name=\"Sheet1\" sheetId=\"1\"/></sheets></workbook>".as_slice()),
            ("xl/worksheets/sheet1.xml", b"<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData></worksheet>".as_slice()),
            ("xl/styles.xml", styles),
        ] {
            zip.start_file(name, options)?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        Ok(())
    }

    fn package_job(root: &Path) -> Result<(MemberJob, Value)> {
        let mut job = provenance_job(root);
        job.mode = JobMode::Compare;
        fs::create_dir_all(&job.work)?;
        package_fixture(&job.work.join("original.xlsx"), b"<styleSheet/>", true)?;
        let baseline = artifact_cache::hash_file(&job.work.join("original.xlsx"))?;
        let mut profiles = Vec::new();
        for name in PROFILES {
            package_fixture(
                &job.work.join(format!("{name}.xlsx")),
                b"<styleSheet/>",
                false,
            )?;
            profiles.push(
                json!({"profile":name,"original_bytes":500,"shrunk_bytes":400,
                "shrink_seconds":1.0,"config_hash":"example","status":"visually unchanged",
                "checks_complete":true,"checked_sheets":1,"unchanged_sheets":1,
                "original_open_seconds":20.0,"shrunk_memory_bytes":12345,
                "open_measurement_provenance":{"old":"measurement"},"value_differences":0}),
            );
        }
        let record = json!({"name":"example.xlsx","baseline_sha256":baseline,
            "baseline_repaired":false,"source_sha256":baseline,"capture_mode":"resident",
            "last_execution":{"capture_mode":"resident"},"status":"comparing","results":profiles,
            "resident_capture":{"peak_job_bytes":12345},
            "comparison_fallback":{"status":"running","reason_code":"IMAGE_BUDGET","original_attempt":{"record_snapshot":"kept.json"}}});
        Ok((job, record))
    }

    #[test]
    fn package_identity_omits_excel_and_preserves_only_real_preparation_measurements() -> Result<()>
    {
        let root = super::super::tests::test_directory("package-no-excel")?;
        let (job, mut record) = package_job(&root)?;
        assert!(finish_unchanged_package(
            &job,
            &mut record,
            &job.work.join("original.xlsx")
        )?);
        assert_eq!(record["status"], "unchanged");
        assert_eq!(record["capture_mode"], "package-audit");
        assert_eq!(record["last_execution"]["capture_mode"], "package-audit");
        assert_eq!(record["package_identity"]["status"], "equivalent");
        assert_eq!(record["whitespace_diagnosis"]["status"], "not_applicable");
        assert!(record["resident_capture"].is_null());
        assert_eq!(
            record["comparison_fallback"]["status"],
            "superseded_by_package_audit"
        );
        assert_eq!(
            record["comparison_fallback"]["original_attempt"]["record_snapshot"],
            "kept.json"
        );
        for profile in record["results"].as_array().unwrap() {
            assert_eq!(profile["status"], "package unchanged");
            assert_eq!(profile["checks_complete"], false);
            assert_eq!(profile["package_checks_complete"], true);
            assert_eq!(profile["shrink_seconds"], 1.0);
            for field in [
                "checked_sheets",
                "unchanged_sheets",
                "original_open_seconds",
                "shrunk_memory_bytes",
                "open_measurement_provenance",
                "value_differences",
            ] {
                assert!(profile[field].is_null(), "stale {field}");
            }
        }
        record["complete"] = json!(true);
        assert!(!super::super::verified_unchanged(&record));
        assert!(!job.work.join("open-original").exists());
        assert!(!job.work.join("resident").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn changed_style_in_one_profile_retains_the_normal_comparison_path() -> Result<()> {
        let root = super::super::tests::test_directory("package-changed-style")?;
        let (job, mut record) = package_job(&root)?;
        package_fixture(
            &job.work.join("maximum-cleaning.xlsx"),
            b"<styleSheet changed=\"1\"/>",
            false,
        )?;
        let previous = record.clone();
        assert!(!finish_unchanged_package(
            &job,
            &mut record,
            &job.work.join("original.xlsx")
        )?);
        assert_eq!(record, previous);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn package_identity_rejects_a_replaced_prepared_baseline() -> Result<()> {
        let root = super::super::tests::test_directory("package-baseline-fingerprint")?;
        let (job, mut record) = package_job(&root)?;
        record["baseline_sha256"] = json!("wrong");
        let error = finish_unchanged_package(&job, &mut record, &job.work.join("original.xlsx"))
            .unwrap_err();
        assert!(error.to_string().contains("baseline fingerprint"));
        assert!(record["package_identity"].is_null());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    pub(super) fn provenance_job(root: &Path) -> MemberJob {
        MemberJob {
            archive: root.join("missing-source.zip"),
            index: 0,
            identity: "example:0:example.xlsx".into(),
            name: "example.xlsx".into(),
            work: root.join("work"),
            record: root.join("records/example.json"),
            report_root: root.into(),
            max_workbook_bytes: 1024,
            mode: JobMode::Prepare,
            bundle: Some(root.join("missing-prepared.zip")),
            resident_capture: false,
            hide_excel: false,
            control_room: None,
            cleanup_barrier: None,
            run_manifest: Some(
                json!({"run_id":"00000000-0000-4000-8000-000000000002","machine_id":"new-pc-b","machine_manifest":"metadata/machines/b.json"}),
            ),
        }
    }

    #[test]
    fn failed_repreparation_retires_the_old_comparison_machine_before_extraction() -> Result<()> {
        let root = super::super::tests::test_directory("retire-old-machine")?;
        let job = provenance_job(&root);
        let old = json!({"run_id":"00000000-0000-4000-8000-000000000001","machine_id":"old-pc-a","machine_manifest":"metadata/machines/a.json"});
        let mut record = json!({"results":[{"profile":"default","original_open_seconds":4.0,"shrunk_open_seconds":2.0}],"whitespace_diagnosis":{"version":1,"status":"complete"}});
        for stage in [
            provenance::Stage::Preparation,
            provenance::Stage::Comparison,
            provenance::Stage::WhitespaceDiagnosis,
        ] {
            provenance::stamp(&mut record, &old, stage);
        }
        assert!(prepare(&job, &mut record, LogLevel::Warn).is_err());
        let saved: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
        assert_eq!(saved["provenance"]["preparation"]["machine_id"], "new-pc-b");
        assert!(saved["provenance"]["comparison"].is_null());
        assert!(saved["provenance"]["whitespace_diagnosis"].is_null());
        assert_eq!(saved["provenance"]["history"].as_array().unwrap().len(), 3);
        assert_eq!(saved["results"], json!([]));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn partial_open_metrics_checkpoint_records_the_new_pair_machine() -> Result<()> {
        let root = super::super::tests::test_directory("partial-open-provenance")?;
        let job = provenance_job(&root);
        let old = json!({"run_id":"00000000-0000-4000-8000-000000000001","machine_id":"old-pc-a"});
        let mut record = json!({"results":[{"profile":"default"},{"profile":"maximum-cleaning"}]});
        provenance::stamp(&mut record, &old, provenance::Stage::Preparation);
        provenance::stamp(&mut record, &old, provenance::Stage::Comparison);
        record_open_metrics(
            &job,
            &mut record,
            "default",
            &json!({"open_seconds":4.0,"working_set_bytes":20}),
            &json!({"open_seconds":2.0,"working_set_bytes":10}),
        )?;
        let saved: Value = serde_json::from_slice(&fs::read(&job.record)?)?;
        assert_eq!(saved["provenance"]["preparation"]["machine_id"], "old-pc-a");
        assert_eq!(saved["provenance"]["comparison"]["machine_id"], "new-pc-b");
        assert_eq!(
            saved["results"][0]["open_measurement_provenance"]["original"]["machine_id"],
            "new-pc-b"
        );
        assert_eq!(provenance::paired_speedup(&saved["results"][0]), Some(2.0));
        assert!(saved["results"][1]["open_measurement_provenance"].is_null());
        assert_eq!(
            saved["provenance"]["history"][0]["reference"]["machine_id"],
            "old-pc-a"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

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

    #[test]
    fn visual_reuse_requires_reproducible_runtime_inputs() {
        let mut manifest: artifact_cache::PackageManifest = serde_json::from_value(json!({
            "archive_sha256":"original", "package_sha256":"package", "parts":{
                "xl/worksheets/sheet1.xml":{"bytes":42,"sha256":"part",
                    "visual_context":false,"visual_runtime":false,"recalc_cacheable":true,"has_formula":false}}})).unwrap();
        assert!(reusable_visual_manifest(&manifest));
        manifest
            .parts
            .get_mut("xl/worksheets/sheet1.xml")
            .unwrap()
            .visual_runtime = true;
        assert!(!reusable_visual_manifest(&manifest));
        manifest
            .parts
            .get_mut("xl/worksheets/sheet1.xml")
            .unwrap()
            .visual_runtime = false;
        manifest
            .parts
            .get_mut("xl/worksheets/sheet1.xml")
            .unwrap()
            .recalc_cacheable = false;
        assert!(!reusable_visual_manifest(&manifest));
        manifest.parts.clear();
        assert!(!reusable_visual_manifest(&manifest));
    }
}
