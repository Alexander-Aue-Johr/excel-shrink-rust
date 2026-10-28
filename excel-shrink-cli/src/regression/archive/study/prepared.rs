//! Shrink serially, then retain the source and distinct outputs in reusable ZIP bundles.
use super::*;
use std::io::Write;
mod package_audit;
mod namespace_exclusion;
mod resident;

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
    // Admission snapshots the controls before starting this workbook worker.
    // Keep its thread count fixed across all three profiles.
    let rayon_threads = preparation_rayon_threads(record, job.control_room.is_some())?;
    record["rayon_threads"] = json!(rayon_threads);
    record["full_scan_deferred"] = json!(false);
    provenance::retire(record, provenance::Stage::Preparation);
    provenance::retire(record, provenance::Stage::Comparison);
    provenance::retire(record, provenance::Stage::WhitespaceDiagnosis);
    record["whitespace_diagnosis"] = json!({"status":"not_investigated"});
    record.as_object_mut().unwrap().remove("package_identity");
    record.as_object_mut().unwrap().remove("namespace_exclusion");
    record.as_object_mut().unwrap().remove("profile_equivalence");
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
    record["converted_sha256"] = json!(artifact_cache::hash_file(&source)?);
    let converted = source.clone();
    record["baseline_sha256"] = record["converted_sha256"].clone();
    record["baseline_repaired"] = json!(false);
    let case: RegressionCase =
        serde_json::from_value(json!({"id":"workbook", "input_path":source}))?;
    let mut outputs = vec![("original".to_owned(), source.clone())];
    record["results"] = json!([]);
    record["shrink_identity"] = json!({});
    record["cleanup_analysis"] = json!({});
    for name in PROFILES {
        work_state::event(
            "shrinking",
            "profile_started",
            "Shrinking profile",
            false,
            None,
            json!({"profile":name}),
        );
        let predefined = predefined_visual_profiles()
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("unknown profile {name}"))?;
        let evidence_path = job.work.join(format!("{name}-shrink-evidence.json"));
        let batch_report_path = job.work.join(format!("{name}-shrink-outcomes.json"));
        let analysis_csv_path = job.work.join(format!("{name}-cleanup.csv"));
        fs::remove_file(&evidence_path).ok();
        fs::remove_file(&batch_report_path).ok();
        fs::remove_file(&analysis_csv_path).ok();
        let mut cli_args = predefined.config.cli_args();
        cli_args.extend([
            "--shrink-evidence-json".to_owned(),
            evidence_path.to_string_lossy().into_owned(),
            "--batch-report".to_owned(),
            batch_report_path.to_string_lossy().into_owned(),
            "--analysis-csv".to_owned(),
            analysis_csv_path.to_string_lossy().into_owned(),
        ]);
        if let Some(threads) = rayon_threads {
            cli_args.extend(["--rayon-threads".to_owned(), threads.to_string()]);
        }
        let report = run_case(
            "study",
            &repo_root(),
            &job.work.join(name),
            None,
            None,
            &cli_args,
            false,
            false,
            &visual_validation::VisualCheckOptions::default(),
            true,
            log_level,
            &case,
        )?;
        if namespace_exclusion::skip_from_batch_report(job, record, &batch_report_path)? {
            return Ok(());
        }
        ensure!(
            report.failures.is_empty(),
            "{name}: {}",
            report.failures.join("; ")
        );
        let evidence: Value = serde_json::from_slice(&fs::read(&evidence_path)?)?;
        let workbooks = evidence["workbooks"].as_array().context("shrink evidence has no workbooks")?;
        ensure!(workbooks.len() == 1, "expected one workbook in shrink evidence");
        let identity = &workbooks[0];
        ensure!(identity["workbook_original_archive_reused"].is_boolean(), "shrink evidence lacks archive reuse result");
        ensure!(identity["worksheet_xml_changed"].is_boolean(), "shrink evidence lacks worksheet change result");
        ensure!(identity["workbook_only_cleanup_available"].is_boolean(), "shrink evidence lacks workbook-only result");
        ensure!(identity["input_file"].as_str().is_some_and(|path| Path::new(path) == source),
            "shrink evidence input does not match converted source");
        ensure!(identity["output_file"].as_str().is_some_and(|path| Path::new(path) == report.output_path),
            "shrink evidence output does not match measured result");
        let cleanup_analysis = if identity["workbook_original_archive_reused"] == true {
            json!({"status":"skipped_unchanged","rows":[],"method":"measured_shrink",
                "matches_measured_output":true})
        } else {
            collect_cleanup_analysis(&analysis_csv_path, &report.output_path)?
        };
        record["shrink_identity"][name] = identity.clone();
        record["cleanup_analysis"][name] = cleanup_analysis.clone();
        record["results"].as_array_mut().unwrap().push(json!({
            "profile":name,"status":"not compared","checks_complete":false,"failures":[],"sheets":[],
            "original_bytes":report.original_bytes,"shrunk_bytes":report.cleaned_bytes,
            "reduction_ratio":report.reduction_ratio,"shrink_seconds":report.shrink_duration_ms.map(|v|v as f64/1000.0),
            "rayon_threads":rayon_threads,
            "config_hash":report.config_hash,"shrink_identity":identity,
            "cleanup_analysis":cleanup_analysis
        }));
        outputs.push((name.into(), report.output_path));
        write_record(&job.record, record)?;
        work_state::event(
            "shrinking",
            "profile_complete",
            "Profile shrunk",
            true,
            None,
            json!({"profile":name}),
        );
    }
    if PROFILES.iter().any(|name| record["shrink_identity"][name]["worksheet_xml_changed"] == true) {
        work_state::event("validating", "baseline_started", "Preparing source baseline", false, None, json!({}));
        let baseline = artifact_cache::prepare_baseline(&source)?;
        record["baseline_sha256"] = json!(baseline.snapshot.manifest.archive_sha256);
        record["baseline_repaired"] = json!(baseline.workbook_xml_changed);
        outputs[0].1 = baseline.snapshot.path;
    }
    // Capture planning and package audits belong to the comparison pass,
    // after every selected workbook has been shrunk.
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

fn preparation_rayon_threads(record: &Value, control_room: bool) -> Result<Option<usize>> {
    if !control_room {
        return Ok(None);
    }
    let value = record["control_room_attempt"]["settings"]["shrink_threads"]
        .as_u64()
        .context("missing admitted shrink thread setting")?;
    let threads = usize::try_from(value).context("shrink thread setting exceeds usize")?;
    let logical_cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    ensure!(
        (1..=logical_cpus).contains(&threads),
        "admitted shrink threads must be between 1 and {logical_cpus}"
    );
    Ok(Some(threads))
}

fn collect_cleanup_analysis(csv: &Path, measured_output: &Path) -> Result<Value> {
    let rows = crate::regression::read_analysis_csv(csv)?;
    ensure!(!rows.is_empty(), "measured shrink produced no cleanup statistics");
    let mut causes = std::collections::BTreeMap::<String, (u64, u64, u64, i64)>::new();
    for row in rows {
        let cause = row.get("cause").context("analysis row lacks cause")?.clone();
        let totals = causes.entry(cause).or_default();
        totals.0 += row["removed_items"].parse::<u64>()?;
        totals.1 += row["removed_bytes_uncompressed_xml"].parse::<u64>()?;
        totals.2 += row["added_bytes_uncompressed_xml"].parse::<u64>()?;
        totals.3 += row["reduction_bytes_uncompressed_xml"].parse::<i64>()?;
    }
    let categories: Vec<_> = causes.into_iter().filter_map(|(cause, totals)| {
        (totals != (0, 0, 0, 0)).then(|| json!({
            "cause":cause,"removed_items":totals.0,
            "removed_bytes_uncompressed_xml":totals.1,
            "added_bytes_uncompressed_xml":totals.2,
            "reduction_bytes_uncompressed_xml":totals.3
        }))
    }).collect();
    Ok(json!({"status":"complete","rows":categories,
        "method":"measured_shrink","matches_measured_output":true,
        "measured_output_bytes":fs::metadata(measured_output)?.len()}))
}

fn save_bundle(
    pending: &Path,
    bundle: &Path,
    outputs: &[(String, PathBuf)],
    converted: &Path,
    record: &mut Value,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(pending)?);
    // XLSX files are already compressed. Recompressing them adds time, not useful space savings.
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut variants = Vec::new();
    let mut stored_members = std::collections::HashMap::<String, String>::new();
    for (name, path) in outputs {
        let sha256 = artifact_cache::hash_file(path)?;
        if name == "original" {
            ensure!(sha256 == record["baseline_sha256"],
                "baseline workbook changed before preparation was saved");
        }
        let member = if let Some(existing) = stored_members.get(&sha256) {
            existing.clone()
        } else {
            let member = format!("{name}.xlsx");
            zip.start_file(&member, options)?;
            std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
            stored_members.insert(sha256.clone(), member.clone());
            member
        };
        if name != "original" {
            let profile = profile_mut(record, name)?;
            profile["output_sha256"] = json!(sha256);
            variants.push(json!({"profile":name,"member":member,
                "sha256":sha256,"config_hash":profile["config_hash"]}));
        }
    }
    let converted_sha256 = artifact_cache::hash_file(converted)?;
    ensure!(
        converted_sha256 == record["converted_sha256"],
        "converted workbook changed before preparation was saved"
    );
    let converted_member = if let Some(existing) = stored_members.get(&converted_sha256) {
        existing.clone()
    } else {
        zip.start_file("converted.xlsx", options)?;
        std::io::copy(&mut fs::File::open(converted)?, &mut zip)?;
        "converted.xlsx".to_owned()
    };
    let metadata = json!({"version":2,"identity":record["identity"],"name":record["name"],
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
    let metadata: Value = serde_json::from_reader(zip.by_name("preparation.json")?)?;
    ensure!(matches!(metadata["version"].as_u64(), Some(1 | 2)),
        "unsupported preparation bundle version");
    let variants = metadata["variants"].as_array().context("preparation bundle lacks variants")?;
    let mut members = std::collections::BTreeMap::from([("original".to_owned(), "original.xlsx".to_owned())]);
    for variant in variants {
        let profile = variant["profile"].as_str().context("preparation variant lacks profile")?;
        let member = variant["member"].as_str().context("preparation variant lacks member")?;
        ensure!(PROFILES.contains(&profile), "unexpected preparation profile {profile}");
        ensure!(member == "original.xlsx" || PROFILES.iter().any(|name| member == format!("{name}.xlsx")),
            "preparation variant refers to unexpected member {member}");
        ensure!(members.insert(profile.to_owned(), member.to_owned()).is_none(),
            "duplicate preparation profile {profile}");
    }
    let mut extracted = std::collections::BTreeMap::<String, PathBuf>::new();
    for name in std::iter::once("original").chain(PROFILES) {
        let filename = format!("{name}.xlsx");
        let member = members.get(name).with_context(|| format!("missing {name} variant"))?;
        let output = work.join(filename);
        if let Some(existing) = extracted.get(member) {
            fs::copy(existing, &output)?;
        } else {
            let mut entry = zip.by_name(member)?;
            std::io::copy(&mut entry, &mut fs::File::create(&output)?)?;
            extracted.insert(member.clone(), output);
        }
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
    // Keep the pending-sheet list until a requested full scan completes.
    // If Excel or the tile budget stops that attempt, the user can retry it.
    if record["full_scan_deferred"] != true {
        record.as_object_mut().unwrap().remove("capture_deferred");
        record.as_object_mut().unwrap().remove("recalc_identity_changed");
    }
    record.as_object_mut().unwrap().remove("full_scan_deferred_completed");
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
    let identities: Vec<_> = record["results"]
        .as_array()
        .context("missing profile measurements")?
        .iter()
        .filter_map(|profile| {
            profile["profile"].as_str().filter(|_| profile["shrink_identity"].is_object())
                .map(|name| (name.to_owned(), profile["shrink_identity"].clone()))
        })
        .collect();
    record["shrink_identity"] = json!({});
    record["cleanup_analysis"] = json!({});
    for (name, identity) in identities {
        record["shrink_identity"][name] = identity;
    }
    let analyses: Vec<_> = record["results"].as_array().unwrap().iter()
        .filter_map(|profile| profile["profile"].as_str()
            .filter(|_| profile["cleanup_analysis"].is_object())
            .map(|name| (name.to_owned(), profile["cleanup_analysis"].clone())))
        .collect();
    for (name, analysis) in analyses {
        record["cleanup_analysis"][name] = analysis;
    }
    if let Some(run) = &job.run_manifest {
        provenance::stamp(record, run, provenance::Stage::Comparison);
    }
    write_record(&job.record, record)?;
    let original = job.work.join("original.xlsx");
    if finish_reused_archives(job, record, &original)? {
        return Ok(());
    }
    verify_partial_reused_archives(job, record, &original)?;
    let comparison_plan = plan_profile_comparisons(job, record, &original)?;
    audit_prepared_profiles(job, record, &original, &comparison_plan)?;
    work_state::event(
        "validating",
        "package_audit",
        "Checking package equivalence",
        false,
        None,
        json!({}),
    );
    if !PROFILES.iter().any(|name| profile_archive_reused(record, name) || profile_workbook_only(record, name))
        && finish_unchanged_package(job, record, &original)?
    {
        return Ok(());
    }
    // Resident clients measure native launch through add-in readiness themselves.
    // Do not open these workbooks again through COM just for dashboard metrics.
    if job.resident_capture {
        return resident::compare(job, record, &comparison_plan);
    }
    // The classic fallback has no native readiness handshake. Leave its startup
    // metrics unavailable rather than launching extra measurement-only Excels.
    for name in PROFILES {
        if profile_workbook_only(record, name) {
            mark_workbook_only(profile_mut(record, name)?);
            write_record(&job.record, record)?;
            continue;
        }
        if profile_archive_reused(record, name) {
            mark_crc_identical(profile_mut(record, name)?);
            write_record(&job.record, record)?;
            continue;
        }
        if comparison_plan.aliases.contains_key(name) {
            continue;
        }
        let result = compare_profile(job, name)?;
        let target = profile_mut(record, name)?;
        for (key, value) in result.as_object().unwrap() {
            target[key] = value.clone();
        }
        comparison_plan.copy_aliases(record, name)?;
        write_record(&job.record, record)?;
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
                    | "shrink_identity"
                    | "cleanup_analysis"
                    | "output_sha256"
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
    record.as_object_mut().unwrap().remove("profile_equivalence");
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

fn profile<'a>(record: &'a Value, name: &str) -> Result<&'a Value> {
    record["results"]
        .as_array()
        .context("missing profile measurements")?
        .iter()
        .find(|profile| profile["profile"] == name)
        .with_context(|| format!("missing {name} measurements"))
}

#[derive(Default)]
struct ProfileComparisonPlan {
    aliases: std::collections::BTreeMap<&'static str, &'static str>,
}

impl ProfileComparisonPlan {
    fn copy_aliases(&self, record: &mut Value, representative: &str) -> Result<()> {
        const PREPARATION_FIELDS: &[&str] = &[
            "profile", "original_bytes", "shrunk_bytes", "reduction_ratio", "shrink_seconds",
            "config_hash", "shrink_identity", "cleanup_analysis", "output_sha256",
            "native_startup", "open_measurement_provenance", "original_open_seconds",
            "shrunk_open_seconds", "original_memory_bytes", "shrunk_memory_bytes",
            "original_private_bytes", "shrunk_private_bytes",
        ];
        let source = profile(record, representative)?.clone();
        let source_fields = source.as_object().context("representative profile is not an object")?;
        for (&alias, &from) in &self.aliases {
            if from != representative {
                continue;
            }
            let target = profile_mut(record, alias)?;
            for (key, value) in source_fields {
                if !PREPARATION_FIELDS.contains(&key.as_str()) {
                    target[key] = value.clone();
                }
            }
            target["comparison_reused_from"] = json!(representative);
            target["comparison_reuse_method"] = json!("identical_archive_sha256_runtime_safe");
            if let Some(sheets) = target["sheets"].as_array_mut() {
                for sheet in sheets {
                    let changed_profiles = sheet
                        .get_mut("capture_coverage")
                        .and_then(|coverage| coverage.get_mut("recalc_changed_profiles"))
                        .and_then(Value::as_array_mut);
                    if let Some(changed_profiles) = changed_profiles {
                        if changed_profiles.iter().any(|name| name == representative)
                            && !changed_profiles.iter().any(|name| name == alias)
                        {
                            changed_profiles.push(json!(alias));
                        }
                    }
                }
            }
            for field in ["capture_deferred", "recalc_identity_changed"] {
                let value = record[field][representative].clone();
                if !value.is_null() {
                    record[field][alias] = value;
                }
            }
            record["profile_equivalence"][alias]["status"] = json!(if source["checks_complete"] == true {
                "reused_complete_result"
            } else {
                "shared_partial_or_failed_result"
            });
        }
        Ok(())
    }

    fn include_startup_aliases(&self, metrics: &mut Value) {
        for (&alias, &representative) in &self.aliases {
            let physical = metrics["capture_client_aliases"][representative]
                .as_str()
                .unwrap_or(representative)
                .to_owned();
            metrics["capture_client_aliases"][alias] = json!(physical);
        }
    }
}

fn plan_profile_comparisons(
    job: &MemberJob,
    record: &mut Value,
    original: &Path,
) -> Result<ProfileComparisonPlan> {
    let mut plan = ProfileComparisonPlan::default();
    let mut first_by_archive = std::collections::HashMap::<String, &'static str>::new();
    let mut first_by_validation =
        std::collections::HashMap::<(String, bool, bool), &'static str>::new();
    let mut first_by_metadata = std::collections::HashMap::<String, &'static str>::new();
    let mut safe_by_archive = std::collections::HashMap::<String, bool>::new();
    let mut original_safe = None;
    let mut notes = serde_json::Map::new();

    for name in PROFILES {
        if profile_archive_reused(record, name) || profile_workbook_only(record, name) {
            notes.insert(name.into(), json!({"status":"excluded"}));
            continue;
        }
        let output = job.work.join(format!("{name}.xlsx"));
        let archive_sha = match profile(record, name)?["output_sha256"].as_str() {
            Some(hash) => hash.to_owned(),
            None => artifact_cache::hash_file(&output)?,
        };
        let config = &predefined_visual_profiles()
            .iter()
            .find(|profile| profile.name == name)
            .expect("all Study profiles are predefined")
            .config;
        // The value checks allow different missing-cell cases per profile.
        // Equal XLSX bytes alone do not make those verdicts interchangeable.
        let validation_key = (
            archive_sha.clone(),
            config.remove_unreferenced_whitespace_cells,
            config.keep_unreferenced_hidden_content_cells,
        );
        let metadata_sha = record["shrink_identity"][name]["cleaned_package_crc_length_sha256"]
            .as_str()
            .map(str::to_owned);
        let mut note = json!({"status":"distinct","archive_sha256":archive_sha});
        if let Some(&representative) = first_by_validation.get(&validation_key) {
            let safe_original = *original_safe.get_or_insert_with(|| {
                artifact_cache::manifest(original)
                    .is_ok_and(|manifest| manifest.archive_sha256 == record["baseline_sha256"]
                        && reusable_visual_manifest(&manifest))
            });
            let safe_output = *safe_by_archive.entry(archive_sha.clone()).or_insert_with(|| {
                artifact_cache::manifest(&output)
                    .is_ok_and(|manifest| manifest.archive_sha256 == archive_sha
                        && reusable_visual_manifest(&manifest))
            });
            if safe_original && safe_output {
                plan.aliases.insert(name, representative);
                note = json!({"status":"alias_planned","representative":representative,
                    "method":"identical_archive_sha256_runtime_safe","archive_sha256":archive_sha});
            } else {
                note = json!({"status":"same_archive_runtime_sensitive",
                    "candidate":representative,"archive_sha256":archive_sha});
            }
        } else if let Some(representative) = first_by_archive.get(&archive_sha) {
            note = json!({"status":"same_archive_different_validation_policy",
                "candidate":representative,"archive_sha256":archive_sha});
        } else if let Some(representative) = metadata_sha.as_ref()
            .and_then(|metadata| first_by_metadata.get(metadata))
        {
            note = json!({"status":"same_crc_length_different_archive",
                "candidate":representative,"archive_sha256":archive_sha});
        }
        first_by_archive.entry(archive_sha).or_insert(name);
        first_by_validation.entry(validation_key).or_insert(name);
        if let Some(metadata) = metadata_sha {
            first_by_metadata.entry(metadata).or_insert(name);
        }
        notes.insert(name.into(), note);
    }
    record["profile_equivalence"] = Value::Object(notes);
    Ok(plan)
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

fn profile_archive_reused(record: &Value, name: &str) -> bool {
    record["shrink_identity"][name]["workbook_original_archive_reused"] == true
}

fn profile_workbook_only(record: &Value, name: &str) -> bool {
    record["shrink_identity"][name]["workbook_only_cleanup_available"] == true
}

fn mark_workbook_only(profile: &mut Value) {
    profile["status"] = json!("workbook-only");
    profile["checks_complete"] = json!(false);
    profile["package_checks_complete"] = json!(true);
    profile["capture_method"] = json!("shrink evidence; worksheet XML unchanged");
    profile["image_comparison"] = json!("not run; only workbook.xml defined names could be cleaned");
    profile["failures"] = json!([]);
    profile["sheets"] = json!([]);
}

fn mark_crc_identical(profile: &mut Value) {
    profile["status"] = json!("identical crc32 and length");
    profile["checks_complete"] = json!(false);
    profile["package_checks_complete"] = json!(true);
    profile["capture_method"] = json!("source archive reused; archive SHA-256 verified");
    profile["image_comparison"] = json!("not run; shrink reused source archive");
    profile["failures"] = json!([]);
    profile["sheets"] = json!([]);
}

/// Finish a preparation whose published outputs contain no worksheet cleanup.
/// This is a terminal shrink outcome, not a visual identity certificate.
pub(super) fn finalize_nonvisual_preparation(record: &mut Value) -> bool {
    if !PROFILES.iter().all(|name| profile_archive_reused(record, name) || profile_workbook_only(record, name)) {
        return false;
    }
    if !record["results"].as_array().is_some_and(|profiles| PROFILES.iter().all(|name| {
        profiles.iter().filter(|profile| profile["profile"] == *name).count() == 1
    })) {
        return false;
    }
    for name in PROFILES {
        let workbook_only = profile_workbook_only(record, name);
        let profile = profile_mut(record, name).expect("all prepared profiles are present");
        if workbook_only {
            mark_workbook_only(profile);
        } else {
            mark_crc_identical(profile);
        }
    }
    record["status"] = json!(if PROFILES.iter().any(|name| profile_workbook_only(record, name)) {
        "workbook-only"
    } else {
        "crc-identical"
    });
    record["checks_complete"] = json!(false);
    record["capture_mode"] = json!("shrink-identity");
    record["error"] = Value::Null;
    record["whitespace_diagnosis"] = json!({"status":"not_applicable",
        "reason":"all cleaning profiles reused the original archive"});
    if record["last_execution"].is_object() {
        record["last_execution"]["capture_mode"] = json!("shrink-identity");
    }
    true
}

fn verify_partial_reused_archives(job: &MemberJob, record: &Value, original: &Path) -> Result<()> {
    if !PROFILES.iter().any(|name| profile_archive_reused(record, name)) {
        return Ok(());
    }
    let baseline_sha = record["baseline_sha256"].as_str().context("missing baseline fingerprint")?;
    ensure!(artifact_cache::hash_file(original)? == baseline_sha,
        "prepared original differs from baseline fingerprint");
    let source_sha = record["converted_sha256"].as_str().context("missing converted fingerprint")?;
    for name in PROFILES {
        if profile_archive_reused(record, name) {
            ensure!(artifact_cache::hash_file(&job.work.join(format!("{name}.xlsx")))? == source_sha,
                "{name}: reused archive differs from converted source");
        }
    }
    Ok(())
}

fn audit_prepared_profiles(
    job: &MemberJob,
    record: &mut Value,
    original: &Path,
    comparison_plan: &ProfileComparisonPlan,
) -> Result<()> {
    let plan = office_plan::build_workbook_visual_plan(original)
        .context("validate workbook capture plan")?;
    ensure!(!plan.sheets.is_empty(), "workbook has no visible worksheets to compare");
    for name in PROFILES {
        let measured_output = job.work.join(format!("{name}.xlsx"));
        if profile_archive_reused(record, name)
            || profile_workbook_only(record, name)
            || comparison_plan.aliases.contains_key(&name)
        {
            continue;
        }
        artifact_cache::assert_shrink_invariants(
            original,
            &measured_output,
            &job.work.join("audit").join(name),
        )?;
        write_record(&job.record, record)?;
    }
    Ok(())
}

// The writer only reuses the whole archive after every planned entry passed
// its unchanged check. Verify the raw archive identity after bundle extraction,
// without decompressing any XML or claiming a visual comparison took place.
fn finish_reused_archives(job: &MemberJob, record: &mut Value, original: &Path) -> Result<bool> {
    if !PROFILES.iter().all(|name| profile_archive_reused(record, name) || profile_workbook_only(record, name)) {
        return Ok(false);
    }
    verify_partial_reused_archives(job, record, original)?;
    ensure!(finalize_nonvisual_preparation(record), "nonvisual preparation classification changed");
    write_record(&job.record, record)?;
    Ok(true)
}

fn compare_profile(job: &MemberJob, name: &str) -> Result<Value> {
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
        sheets: None,
        keep_screenshots: false,
        combined_report: Some(job.report_root.join("report.html")),
    };
    let mut result = json!({"profile":name,"status":"technical error","checks_complete":false,"sheets":[],"requested_sheets":null});
    let checked = (|| -> Result<Vec<String>> {
        work_state::event("validating_values", "classic_recalculation_started",
            "Checking cell values before classic capture", false, Some("Excel recalculation"),
            json!({"profile":name}));
        let (values, mut failures) =
            validate_excel_recalculation(&original, &shrunk, &directory, &cli)?;
        work_state::event("comparing", "classic_recalculation_complete",
            "Cell values checked; preparing classic comparison", true, None,
            json!({"profile":name}));
        // Value differences are recorded alongside the full image comparison.
        let original_sha = artifact_cache::hash_file(&original)?;
        let shrunk_sha = artifact_cache::hash_file(&shrunk)?;
        work_state::event("waiting_classic", "classic_capture_started",
            "Classic comparison subprocess running; inspect its Excel workers for progress", false,
            Some("classic capture process"), json!({"profile":name}));
        let captured = visual_validation::compare_sheet_visuals(
            &original,
            &shrunk,
            &directory.join("visual"),
            &options,
        )?;
        let mut visual: Value =
            serde_json::from_slice(&fs::read(captured.output_dir.join("visual-result.json"))?)?;
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
        Ok(failures)
    })();
    result["profile"] = json!(name);
    result["requested_sheets"] = Value::Null;
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
        items.retain(|s| s["status"] != "same");
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
    fn preparation_uses_admitted_thread_count_for_the_workbook() -> Result<()> {
        let admitted = json!({"control_room_attempt":{"settings":{"shrink_threads":1}}});
        assert_eq!(preparation_rayon_threads(&admitted, true)?, Some(1));
        assert_eq!(preparation_rayon_threads(&admitted, false)?, None);
        assert!(preparation_rayon_threads(&json!({}), true).is_err());
        Ok(())
    }

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
                &mut record,
            )?;
            fs::remove_dir_all(&job.work)?;

            let mut zip = ZipArchive::new(fs::File::open(&bundle)?)?;
            let metadata: Value = serde_json::from_reader(zip.by_name("preparation.json")?)?;
            assert_eq!(metadata["source_sha256"], "source-bytes-hash");
            assert_eq!(metadata["version"], 2);
            assert_eq!(metadata["converted_sha256"], record["converted_sha256"]);
            assert_eq!(metadata["baseline_sha256"], record["baseline_sha256"]);
            assert_eq!(metadata["prepared_by"], "producer-binary-hash");
            assert_eq!(metadata["provenance"]["machine_id"], "conversion-pc");
            let converted_member = if repaired {
                "default.xlsx"
            } else {
                "original.xlsx"
            };
            assert_eq!(metadata["converted_member"], converted_member);
            assert_eq!(zip.len(), 4);
            for profile in metadata["variants"].as_array().unwrap() {
                assert_eq!(profile["member"], "default.xlsx");
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
            for variant in metadata["variants"].as_array().unwrap() {
                let profile = variant["profile"].as_str().unwrap();
                assert_eq!(artifact_cache::hash_file(&reopened.join(format!("{profile}.xlsx")))?,
                    variant["sha256"]);
            }
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }

    #[test]
    fn identical_outputs_share_comparison_only_with_matching_value_policy() -> Result<()> {
        let root = super::super::tests::test_directory("profile-comparison-aliases")?;
        let (job, mut record) = package_job(&root)?;
        for name in PROFILES {
            let hash = artifact_cache::hash_file(&job.work.join(format!("{name}.xlsx")))?;
            profile_mut(&mut record, name)?["output_sha256"] = json!(hash);
            record["shrink_identity"][name] = json!({
                "workbook_original_archive_reused":false,
                "workbook_only_cleanup_available":false,
            });
        }
        profile_mut(&mut record, "maximum-cleaning")?["config_hash"] = json!("maximum-config");
        let plan = plan_profile_comparisons(&job, &mut record, &job.work.join("original.xlsx"))?;
        assert!(!plan.aliases.contains_key("maximum-cleaning"));
        assert_eq!(plan.aliases["too-aggressive-cleaning"], "maximum-cleaning");
        assert_eq!(record["profile_equivalence"]["maximum-cleaning"]["status"],
            "same_archive_different_validation_policy");
        assert_eq!(record["profile_equivalence"]["too-aggressive-cleaning"]["status"], "alias_planned");

        let representative = profile_mut(&mut record, "maximum-cleaning")?;
        representative["status"] = json!("paused for later full scan");
        representative["checks_complete"] = json!(false);
        representative["sheets"] = json!([{"index":1,"status":"same",
            "capture_coverage":{"deferred_full_scan":true,
                "recalc_identity_changed":true,
                "recalc_changed_profiles":["maximum-cleaning"]}}]);
        representative["value_differences"] = json!(2);
        record["capture_deferred"]["maximum-cleaning"] = json!([1]);
        record["recalc_identity_changed"]["maximum-cleaning"] = json!([1]);
        plan.copy_aliases(&mut record, "maximum-cleaning")?;
        let aggressive = profile(&record, "too-aggressive-cleaning")?;
        assert_eq!(profile(&record, "maximum-cleaning")?["config_hash"], "maximum-config");
        assert_eq!(aggressive["status"], "paused for later full scan");
        assert_eq!(aggressive["value_differences"], 2);
        assert_eq!(aggressive["comparison_reused_from"], "maximum-cleaning");
        assert_eq!(record["capture_deferred"]["too-aggressive-cleaning"], json!([1]));
        assert_eq!(record["recalc_identity_changed"]["too-aggressive-cleaning"], json!([1]));
        assert_eq!(aggressive["sheets"][0]["capture_coverage"]["recalc_changed_profiles"],
            json!(["maximum-cleaning", "too-aggressive-cleaning"]));

        // A failed representative cannot turn an alias into a completed visual check.
        let representative = profile_mut(&mut record, "maximum-cleaning")?;
        representative["status"] = json!("technical error");
        representative["checks_complete"] = json!(false);
        representative["failures"] = json!(["capture failed"]);
        plan.copy_aliases(&mut record, "maximum-cleaning")?;
        let alias = profile(&record, "too-aggressive-cleaning")?;
        assert_eq!(alias["status"], "technical error");
        assert_eq!(alias["checks_complete"], false);
        assert_eq!(alias["failures"], json!(["capture failed"]));
        assert_eq!(record["profile_equivalence"]["too-aggressive-cleaning"]["status"],
            "shared_partial_or_failed_result");

        let mut metrics = json!({"capture_client_aliases":{"maximum-cleaning":"physical-maximum"}});
        plan.include_startup_aliases(&mut metrics);
        assert_eq!(metrics["capture_client_aliases"]["too-aggressive-cleaning"], "physical-maximum");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn matching_package_crc_metadata_does_not_alias_different_xlsx_bytes() -> Result<()> {
        let root = super::super::tests::test_directory("profile-metadata-only")?;
        let (job, mut record) = package_job(&root)?;
        fs::copy(job.work.join("original.xlsx"), job.work.join("maximum-cleaning.xlsx"))?;
        for name in PROFILES {
            let hash = artifact_cache::hash_file(&job.work.join(format!("{name}.xlsx")))?;
            profile_mut(&mut record, name)?["output_sha256"] = json!(hash);
            record["shrink_identity"][name] = json!({
                "workbook_original_archive_reused":false,
                "workbook_only_cleanup_available":false,
                "cleaned_package_crc_length_sha256":"same-member-metadata",
            });
        }
        let plan = plan_profile_comparisons(&job, &mut record, &job.work.join("original.xlsx"))?;
        assert!(!plan.aliases.contains_key("maximum-cleaning"));
        assert_eq!(record["profile_equivalence"]["maximum-cleaning"]["status"],
            "same_crc_length_different_archive");
        assert!(!plan.aliases.contains_key("too-aggressive-cleaning"));
        assert_eq!(record["profile_equivalence"]["too-aggressive-cleaning"]["status"],
            "same_archive_different_validation_policy");
        fs::remove_dir_all(root)?;
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
    fn reused_archives_skip_xml_and_visual_comparison_without_claiming_visual_identity() -> Result<()> {
        let root = super::super::tests::test_directory("reused-archive-identity")?;
        let (job, mut record) = package_job(&root)?;
        let original = job.work.join("original.xlsx");
        record["converted_sha256"] = record["baseline_sha256"].clone();
        for name in PROFILES {
            fs::copy(&original, job.work.join(format!("{name}.xlsx")))?;
            record["shrink_identity"][name] = json!({
                "workbook_original_archive_reused":true,
                "workbook_crc_length_equal":true,
            });
        }
        assert!(finish_reused_archives(&job, &mut record, &original)?);
        assert_eq!(record["status"], "crc-identical");
        assert_eq!(record["checks_complete"], false);
        assert_eq!(record["capture_mode"], "shrink-identity");
        assert!(record["package_identity"].is_null());
        for profile in record["results"].as_array().unwrap() {
            assert_eq!(profile["status"], "identical crc32 and length");
            assert_eq!(profile["checks_complete"], false);
            assert_eq!(profile["package_checks_complete"], true);
        }
        fs::write(job.work.join("default.xlsx"), b"changed")?;
        assert!(finish_reused_archives(&job, &mut record, &original).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn workbook_only_cleanup_skips_baseline_and_visual_comparison() -> Result<()> {
        let root = super::super::tests::test_directory("workbook-only-identity")?;
        let (job, mut record) = package_job(&root)?;
        let original = job.work.join("original.xlsx");
        record["converted_sha256"] = record["baseline_sha256"].clone();
        for name in PROFILES {
            fs::copy(&original, job.work.join(format!("{name}.xlsx")))?;
            record["shrink_identity"][name] = json!({
                "workbook_original_archive_reused":true,
                "workbook_only_cleanup_available":true,
                "worksheet_xml_changed":false,
            });
        }
        assert!(finish_reused_archives(&job, &mut record, &original)?);
        assert_eq!(record["status"], "workbook-only");
        assert!(record["results"].as_array().unwrap().iter().all(|profile| {
            profile["status"] == "workbook-only" && profile["checks_complete"] == false
        }));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn measured_analysis_records_category_provenance_without_replacing_output() -> Result<()> {
        let root = super::super::tests::test_directory("measured-cleanup-analysis")?;
        let (job, _) = package_job(&root)?;
        let measured = job.work.join("default.xlsx");
        let before = artifact_cache::hash_file(&measured)?;
        let csv = root.join("cleanup.csv");
        fs::write(&csv, concat!(
            "input_file,sheet_file,cause,removed_items,removed_bytes_uncompressed_xml,added_bytes_uncompressed_xml,reduction_bytes_uncompressed_xml,reduction_ratio,original_sheet_xml_bytes,cleaned_sheet_xml_bytes,sheet_reduction_bytes_uncompressed_xml,sheet_reduction_ratio,tail_pruning_disabled\n",
            "source.xlsx,xl/worksheets/sheet1.xml,EmptyCellsWithoutContent,2,20,0,20,0.2,100,80,20,0.2,true\n",
        ))?;
        let analysis = collect_cleanup_analysis(&csv, &measured)?;
        assert_eq!(analysis["status"], "complete", "{analysis}");
        assert_eq!(analysis["method"], "measured_shrink");
        assert_eq!(analysis["rows"][0]["removed_items"], 2);
        assert_eq!(analysis["matches_measured_output"], true);
        assert_eq!(artifact_cache::hash_file(&measured)?, before);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn measured_analysis_rejects_an_empty_report() -> Result<()> {
        let root = super::super::tests::test_directory("empty-measured-analysis")?;
        let (job, _) = package_job(&root)?;
        let csv = root.join("cleanup.csv");
        fs::write(&csv, "input_file,sheet_file,cause\n")?;
        assert!(collect_cleanup_analysis(&csv, &job.work.join("default.xlsx")).is_err());
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
