//! Study integration for one resident original and its three profile workbooks.
use super::*;
use crate::regression::office_capture::ResidentCapture;
use std::collections::BTreeMap;

pub(super) fn compare(job: &MemberJob, record: &mut Value) -> Result<()> {
    let original = job.work.join("original.xlsx");
    let original_sha = artifact_cache::hash_file(&original)?;
    // Finish the existing recalculation checks before opening the resident set.
    // Their independent Excel instances would otherwise compete with capture.
    let mut validations = BTreeMap::new();
    let mut recalc_inputs = json!({"variants":{}});
    for name in PROFILES {
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
    let variants: Vec<_> = PROFILES
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
    fs::write(
        resident_dir.join("recalc-inputs.json"),
        serde_json::to_vec(&recalc_inputs)?,
    )?;
    let mut session = ResidentCapture::start(&original, &variants, &resident_dir)?;
    for name in ["too-aggressive-cleaning", "maximum-cleaning", "default"] {
        work_state::event(
            "comparing",
            "comparison_started",
            "Comparing profile images",
            false,
            None,
            json!({"profile":name}),
        );
        let output = job.work.join("comparisons").join(name).join("visual");
        let mut visual = session.compare(name, &output)?;
        let mut failures = validations.remove(name).unwrap();
        let sheets = visual["sheets"]
            .as_array()
            .context("missing resident sheets")?;
        let changed = sheets.iter().any(|s| s["status"] == "changed");
        let sampled = sheets
            .iter()
            .any(|s| s["capture_coverage"]["omitted_tiles"].as_u64().unwrap_or(0) > 0);
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
        } else if sampled {
            "checked ranges visually unchanged; tall-sheet ranges omitted"
        } else {
            "visually unchanged"
        });
        visual["checks_complete"] = json!(true);
        visual["requested_sheets"] = Value::Null;
        visual["failures"] = json!(failures);
        let target = profile_mut(record, name)?;
        for (key, value) in visual.as_object().unwrap() {
            target[key] = value.clone();
        }
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
    record["status"] = json!(if changed { "differences" } else { "unchanged" });
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
    for name in PROFILES {
        session.close(name)?;
    }
    match session.finish() {
        Ok(metrics) => record["resident_capture"] = metrics,
        Err(error) if record["whitespace_diagnosis"]["status"] == "failed" => {
            // Completed primary checks remain usable if an auxiliary control fails.
            record["resident_capture"] =
                json!({"status":"control failed","error":format!("{error:#}")});
        }
        Err(error) => return Err(error),
    }
    for profile in record["results"].as_array_mut().unwrap() {
        if let Some(sheets) = profile["sheets"].as_array_mut() {
            sheets.retain(|s| s["status"] != "same");
        }
    }
    write_record(&job.record, record)
}
