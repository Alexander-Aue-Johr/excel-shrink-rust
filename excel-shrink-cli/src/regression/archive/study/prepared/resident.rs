//! Study integration for one resident original and its three profile workbooks.
use super::*;
use crate::regression::office_capture::ResidentCapture;
use std::collections::BTreeMap;
mod startup;

pub(super) fn compare(
    job: &MemberJob,
    record: &mut Value,
    comparison_plan: &ProfileComparisonPlan,
) -> Result<()> {
    let original = job.work.join("original.xlsx");
    let original_sha = artifact_cache::hash_file(&original)?;
    let active_profiles: Vec<_> = PROFILES
        .into_iter()
        .filter(|name| !profile_archive_reused(record, name)
            && !profile_workbook_only(record, name)
            && !comparison_plan.aliases.contains_key(name))
        .collect();
    for name in PROFILES {
        if profile_workbook_only(record, name) {
            mark_workbook_only(profile_mut(record, name)?);
        } else if profile_archive_reused(record, name) {
            mark_crc_identical(profile_mut(record, name)?);
        }
    }
    write_record(&job.record, record)?;
    // Finish the existing recalculation checks before opening the resident set.
    // Their independent Excel instances would otherwise compete with capture.
    let mut validations = BTreeMap::new();
    let mut recalc_inputs = json!({"variants":{}});
    for name in active_profiles.iter().copied() {
        work_state::event(
            "validating_values",
            "recalculation_started",
            "Validating stored and recalculated cell values",
            false,
            Some("Excel recalculation validation"),
            json!({"profile":name}),
        );
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
        let shrunk = job.work.join(format!("{name}.xlsx"));
        let (values, failures) =
            validate_excel_recalculation(&original, &shrunk, &directory, &cli)?;
        recalc_inputs["variants"][name] = json!(values.shrunk_recalculated.path);
        if let Some(reference) = &values.original_recalculated {
            recalc_inputs["original"] = json!(reference.path);
        }
        let target = profile_mut(record, name)?;
        target["value_differences"] = json!(
            values.original_vs_shrunk.difference_count + values.real_recalc_differences.len()
        );
        target["stored_value_differences"] = json!(values.original_vs_shrunk.difference_count);
        target["recalculated_value_differences"] = json!(values.real_recalc_differences.len());
        target["comparison_input_sha256"] =
            json!({"original":original_sha,"shrunk":artifact_cache::hash_file(&shrunk)?});
        validations.insert(name, failures);
        write_record(&job.record, record)?;
        work_state::event(
            "validating_values",
            "recalculation_complete",
            "Cell values validated",
            true,
            None,
            json!({"profile":name}),
        );
    }
    let variants: Vec<_> = active_profiles
        .iter()
        .map(|name| (*name, job.work.join(format!("{name}.xlsx"))))
        .collect();
    work_state::event(
        "opening",
        "resident_started",
        "Starting resident Excel capture clients",
        false,
        Some("resident Excel clients"),
        json!({}),
    );
    let resident_dir = job.work.join("resident");
    fs::create_dir_all(&resident_dir)?;
    recalc_inputs["shrink_identity"] = record["shrink_identity"].clone();
    recalc_inputs["full_scan_deferred"] = record["full_scan_deferred"].clone();
    fs::write(
        resident_dir.join("recalc-inputs.json"),
        serde_json::to_vec(&recalc_inputs)?,
    )?;
    let mut session = ResidentCapture::start(&original, &variants, &resident_dir)?;
    for name in ["too-aggressive-cleaning", "maximum-cleaning", "default"] {
        if !active_profiles.contains(&name) {
            continue;
        }
        work_state::event(
            "comparing",
            "comparison_started",
            "Comparing profile images",
            false,
            None,
            json!({"profile":name}),
        );
        let output = job.work.join("comparisons").join(name).join("visual");
        let compared = session.compare(name, &output);
        // Persist completed startup samples even if capture or cleanup fails.
        // A metrics read must not hide the original comparison failure.
        match session.startup_metrics() {
            Ok(Some(mut metrics)) => {
                comparison_plan.include_startup_aliases(&mut metrics);
                startup::record(record, &metrics);
                write_record(&job.record, record)?;
            }
            Ok(None) => {}
            Err(error) => eprintln!("Resident startup snapshot unavailable: {error:#}"),
        }
        let mut visual = compared?;
        let mut failures = validations.remove(name).unwrap();
        let sheets = visual["sheets"]
            .as_array()
            .context("missing resident sheets")?;
        let changed = sheets.iter().any(|s| s["status"] == "changed");
        let sampled = sheets
            .iter()
            .any(|s| s["capture_coverage"]["omitted_tiles"].as_u64().unwrap_or(0) > 0);
        let deferred_sheets: Vec<_> = sheets.iter()
            .filter(|sheet| sheet["capture_coverage"]["deferred_full_scan"] == true)
            .filter_map(|sheet| sheet["index"].as_u64())
            .collect();
        let recalc_drift_sheets: Vec<_> = sheets.iter()
            .filter(|sheet| sheet["capture_coverage"]["recalc_identity_changed"] == true)
            .filter(|sheet| sheet["capture_coverage"]["recalc_changed_profiles"]
                .as_array()
                .is_some_and(|profiles| profiles.iter().any(|profile| profile.as_str() == Some(name))))
            .filter_map(|sheet| sheet["index"].as_u64())
            .collect();
        record["capture_deferred"][name] = json!(deferred_sheets);
        record["recalc_identity_changed"][name] = json!(recalc_drift_sheets);
        visual["checked_sheets"] = json!(sheets.len());
        visual["unchanged_sheets"] = json!(visual["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["status"] == "same")
            .count());
        if changed {
            failures.push("Visual differences found".into());
        }
        visual["status"] = json!(if changed {
            "visual differences"
        } else if !failures.is_empty() {
            "cell value differences"
        } else if !deferred_sheets.is_empty() {
            "paused for later full scan"
        } else if sampled {
            "checked ranges visually unchanged; tall-sheet ranges omitted"
        } else {
            "visually unchanged"
        });
        visual["checks_complete"] = json!(!sampled);
        visual["requested_sheets"] = Value::Null;
        visual["failures"] = json!(failures);
        let target = profile_mut(record, name)?;
        for (key, value) in visual.as_object().unwrap() {
            target[key] = value.clone();
        }
        comparison_plan.copy_aliases(record, name)?;
        write_record(&job.record, record)?;
        work_state::event(
            "comparing",
            "comparison_complete",
            "Profile comparison saved",
            true,
            None,
            json!({"profile":name}),
        );
    }
    // Controls can be byte-identical to a primary profile. Keep those clients
    // available until the controls have either reused or completed their captures.
    let changed = record["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["failures"].as_array().is_some_and(|a| !a.is_empty()));
    let deferred = record["capture_deferred"].as_object().is_some_and(|profiles| profiles.values()
        .any(|sheets| sheets.as_array().is_some_and(|items| !items.is_empty())));
    if record["full_scan_deferred"] == true {
        record["full_scan_deferred_completed"] = json!(!deferred);
    }
    record["status"] = json!(if changed { "differences" } else if deferred { "paused-for-later" } else { "unchanged" });
    record["capture_policy"] = json!("tall-sheet-v1");
    record["visual_coverage"] = json!(if record["results"].as_array().unwrap().iter().any(|p| p
        ["sheets"]
        .as_array()
        .is_some_and(|sheets| sheets
            .iter()
            .any(|s| s["capture_coverage"]["omitted_tiles"].as_u64().unwrap_or(0) > 0)))
    {
        "sampled"
    } else {
        "full"
    });
    if changed {
        work_state::event(
            "whitespace_diagnosis",
            "diagnosis_started",
            "Checking whitespace counterfactuals",
            false,
            None,
            json!({}),
        );
        super::super::whitespace::run_resident(job, record, LogLevel::Warn, &mut session);
    }
    write_record(&job.record, record)?;
    for name in active_profiles {
        session.close(name)?;
    }
    match session.finish() {
        Ok(mut metrics) => {
            comparison_plan.include_startup_aliases(&mut metrics);
            startup::record(record, &metrics);
            record["resident_capture"] = metrics;
        }
        Err(error) if record["whitespace_diagnosis"]["status"] == "failed" => {
            // Completed primary checks remain usable if an auxiliary control fails.
            record["resident_capture"] =
                json!({"status":"control failed","error":format!("{error:#}")});
        }
        Err(error) => return Err(error),
    }
    for profile in record["results"].as_array_mut().unwrap() {
        if let Some(sheets) = profile["sheets"].as_array_mut() {
            sheets.retain(|s| s["status"] != "same" || s["capture_coverage"]["deferred_full_scan"] == true);
        }
    }
    write_record(&job.record, record)
}
