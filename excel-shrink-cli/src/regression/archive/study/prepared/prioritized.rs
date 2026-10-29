//! Aggressive-first screening with targeted follow-up, followed by a separate full pass.
use super::super::strategy;
use super::*;
use crate::regression::office_capture::ResidentCapture;
use std::collections::BTreeMap;

fn validate_profile(
    job: &MemberJob,
    record: &mut Value,
    name: &str,
    inputs: &mut Value,
) -> Result<Vec<String>> {
    work_state::event(
        "validating_values",
        "recalculation_started",
        "Validating selected profile",
        false,
        Some("Excel recalculation validation"),
        json!({"profile":name}),
    );
    let original = job.work.join("original.xlsx");
    let shrunk = job.work.join(format!("{name}.xlsx"));
    let directory = job.work.join("comparisons").join(name);
    fs::create_dir_all(&directory)?;
    let config = &predefined_visual_profiles()
        .iter()
        .find(|profile| profile.name == name)
        .context("unknown comparison profile")?
        .config;
    let case: RegressionCase =
        serde_json::from_value(json!({"id":"workbook","input_path":original}))?;
    let cli = build_cli(
        &original,
        &directory,
        None,
        &case,
        false,
        &directory.join("analysis.csv"),
        &config.cli_args(),
    );
    let (values, failures) = validate_excel_recalculation(&original, &shrunk, &directory, &cli)?;
    inputs["variants"][name] = json!(values.shrunk_recalculated.path);
    if let Some(reference) = values.original_recalculated {
        inputs["original"] = json!(reference.path);
    }
    let target = profile_mut(record, name)?;
    target["value_differences"] =
        json!(values.original_vs_shrunk.difference_count + values.real_recalc_differences.len());
    target["stored_value_differences"] = json!(values.original_vs_shrunk.difference_count);
    target["recalculated_value_differences"] = json!(values.real_recalc_differences.len());
    target["comparison_input_sha256"] = json!({"original":artifact_cache::hash_file(&original)?,"shrunk":artifact_cache::hash_file(&shrunk)?});
    write_record(&job.record, record)?;
    Ok(failures)
}

fn changed(visual: &Value) -> bool {
    visual["sheets"]
        .as_array()
        .is_some_and(|sheets| sheets.iter().any(|sheet| sheet["status"] == "changed"))
}

fn save_visual(
    record: &mut Value,
    name: &str,
    mut visual: Value,
    mut failures: Vec<String>,
    targeted: bool,
) -> Result<()> {
    let difference = changed(&visual);
    if difference {
        failures.push("Visual differences found".into());
    }
    let sheets = visual["sheets"]
        .as_array()
        .context("missing profile sheets")?;
    let omitted = sheets.iter().any(|sheet| {
        sheet["capture_coverage"]["omitted_tiles"]
            .as_u64()
            .unwrap_or(0)
            > 0
    });
    let checked = sheets.len();
    let unchanged = sheets
        .iter()
        .filter(|sheet| sheet["status"] == "same")
        .count();
    visual["checked_sheets"] = json!(checked);
    visual["unchanged_sheets"] = json!(unchanged);
    visual["checks_complete"] = json!(!targeted && !omitted);
    visual["targeted_checks_complete"] = json!(targeted && !omitted);
    visual["requested_sheets"] = if targeted {
        json!("aggressive mismatch regions")
    } else {
        Value::Null
    };
    visual["coverage_scope"] = json!(if targeted {
        "aggressive mismatch regions only"
    } else {
        "full selected profile"
    });
    visual["status"] = json!(if difference {
        "visual differences"
    } else if !failures.is_empty() {
        "cell value differences"
    } else if targeted {
        "selected regions visually unchanged"
    } else if omitted {
        "checked ranges visually unchanged; ranges omitted"
    } else {
        "visually unchanged"
    });
    visual["failures"] = json!(failures);
    let target = profile_mut(record, name)?;
    for (key, value) in visual.as_object().unwrap() {
        target[key] = value.clone();
    }
    Ok(())
}

fn set_comparison_status(record: &mut Value) {
    let differences = record["results"].as_array().into_iter().flatten().any(|profile| {
        changed(profile)
            || profile["failures"].as_array().is_some_and(|errors| !errors.is_empty())
    });
    record["status"] = json!(if differences { "differences" } else { "unchanged" });
}

fn finish_without_capture(record: &mut Value) {
    // The secondary pass may reuse an aggressive difference even when every
    // newly selected profile is package-identical and needs no Excel client.
    set_comparison_status(record);
    record["capture_mode"] = json!("package-audit");
    record["visual_coverage"] = json!(if strategy::pass(record) == strategy::PRIMARY {
        "aggressive profile plus targeted follow-up"
    } else {
        "all profiles"
    });
    record["whitespace_diagnosis"] = json!({"version":super::super::whitespace::VERSION,
        "status":"not_applicable","reason":"selected profiles require no visual capture"});
}

pub(super) fn compare(job: &MemberJob, record: &mut Value) -> Result<()> {
    let primary = strategy::pass(record) == strategy::PRIMARY;
    let selected = strategy::profiles(record).to_vec();
    let active: Vec<_> = selected
        .into_iter()
        .filter(|name| {
            !profile_archive_reused(record, name) && !profile_workbook_only(record, name)
        })
        .collect();
    for name in PROFILES {
        if profile_workbook_only(record, name) {
            mark_workbook_only(profile_mut(record, name)?);
        } else if profile_archive_reused(record, name) {
            mark_crc_identical(profile_mut(record, name)?);
        }
    }
    if active.is_empty() {
        finish_without_capture(record);
        return write_record(&job.record, record);
    }
    let mut inputs = json!({"variants":{}});
    let mut validations = BTreeMap::new();
    for name in &active {
        validations.insert(*name, validate_profile(job, record, name, &mut inputs)?);
    }
    let directory = job.work.join("resident");
    fs::create_dir_all(&directory)?;
    inputs["shrink_identity"] = record["shrink_identity"].clone();
    inputs["full_scan_deferred"] = record["full_scan_deferred"].clone();
    fs::write(
        directory.join("recalc-inputs.json"),
        serde_json::to_vec(&inputs)?,
    )?;
    let variants: Vec<_> = active
        .iter()
        .map(|name| (*name, job.work.join(format!("{name}.xlsx"))))
        .collect();
    let mut session =
        ResidentCapture::start(&job.work.join("original.xlsx"), &variants, &directory)?;
    let mut captured = Vec::new();
    let mut aggressive_visual = None;
    for name in &active {
        let output = job.work.join("comparisons").join(name).join("visual");
        let visual = session.compare(name, &output)?;
        if *name == strategy::AGGRESSIVE {
            aggressive_visual = Some(visual.clone());
        }
        save_visual(
            record,
            name,
            visual,
            validations.remove(name).unwrap_or_default(),
            false,
        )?;
        captured.push(*name);
        write_record(&job.record, record)?;
    }
    if primary {
        if let Some(visual) = aggressive_visual.as_ref().filter(|visual| changed(visual)) {
            for name in ["default", "maximum-cleaning"] {
                if profile_archive_reused(record, name) || profile_workbook_only(record, name) {
                    continue;
                }
                // These are targeted raw-input captures, not full value checks.
                // The second global pass performs the existing recalc validation.
                let failures = Vec::new();
                profile_mut(record, name)?["recalculation_validation"] =
                    json!("deferred_to_full_pass");
                session.add_profile(name, &job.work.join(format!("{name}.xlsx")))?;
                let output = job.work.join("comparisons").join(name).join("visual");
                let result = session.compare_selected(name, &output, visual)?;
                save_visual(record, name, result, failures, true)?;
                captured.push(name);
                write_record(&job.record, record)?;
            }
        }
    }
    // Original evidence and all profile captures are fixed before any repair.
    let mut diagnosed_clients = BTreeMap::<String, String>::new();
    for name in &captured {
        let visual = profile(record, name)?.clone();
        if !changed(&visual) {
            continue;
        }
        let physical = visual["shared_capture_client"]
            .as_str()
            .unwrap_or(name)
            .to_owned();
        if let Some(previous) = diagnosed_clients.get(&physical) {
            let mut diagnosis = profile(record, previous)?["visual_diagnosis"].clone();
            diagnosis["reused_from_profile"] = json!(previous);
            profile_mut(record, name)?["visual_diagnosis"] = diagnosis;
            continue;
        }
        diagnosed_clients.insert(physical, (*name).to_owned());
        let output = job.work.join("comparisons").join(name).join("diagnosis");
        let diagnosis = match session.diagnose_profile(name, &visual, &output) {
            Ok(diagnosis) => diagnosis,
            Err(error) => {
                json!({"status":"unresolved","category":"unresolved","error":format!("{error:#}"),"scope":"primary mismatch tiles"})
            }
        };
        profile_mut(record, name)?["visual_diagnosis"] = diagnosis;
        write_record(&job.record, record)?;
    }
    set_comparison_status(record);
    record["capture_policy"] = json!("aggressive-first-autonomous-sweep-v1");
    record["visual_coverage"] = json!(if primary {
        "aggressive profile plus targeted follow-up"
    } else {
        "all profiles"
    });
    record["whitespace_diagnosis"] = json!({"version":super::super::whitespace::VERSION,
        "status":"handled_by_visual_diagnosis","method":"ordered in-memory counterfactuals"});
    let mut cleanup_errors = Vec::new();
    for name in captured {
        if let Err(error) = session.close(name) {
            cleanup_errors.push(format!("close {name}: {error:#}"));
        }
    }
    match session.finish() {
        Ok(metrics) => {
            super::resident::startup::record(record, &metrics);
            record["resident_capture"] = metrics;
        }
        Err(error) => {
            cleanup_errors.push(format!("finish optional controls: {error:#}"));
            record["resident_capture"] =
                json!({"status":"auxiliary_failure","error":format!("{error:#}")});
        }
    }
    if !cleanup_errors.is_empty() {
        record["auxiliary_capture_errors"] = json!(cleanup_errors);
    }
    write_record(&job.record, record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_only_secondary_preserves_reused_aggressive_differences() {
        for (failures, sheets) in [
            (json!(["Visual differences found"]), json!([])),
            (json!([]), json!([{"status":"changed"}])),
        ] {
            let profiles = json!([
                {"profile":"default","package_checks_complete":true},
                {"profile":"maximum-cleaning","package_checks_complete":true},
                {"profile":strategy::AGGRESSIVE,"checks_complete":true,
                    "comparison_reused_from_pass":strategy::PRIMARY,"failures":failures,"sheets":sheets,
                    "visual_diagnosis":{"category":"row_height","repair_image":"evidence/repair.png"}}
            ]);
            let mut record = json!({"comparison_pass":strategy::SECONDARY,"results":profiles});
            finish_without_capture(&mut record);
            assert_eq!(record["status"], "differences");
            assert_eq!(record["capture_mode"], "package-audit");
            assert_eq!(record["visual_coverage"], "all profiles");
            assert_eq!(record["results"], profiles);
        }
    }

    #[test]
    fn package_only_secondary_remains_unchanged_without_profile_differences() {
        let mut record = json!({"comparison_pass":strategy::SECONDARY,"results":[
            {"profile":"default","package_checks_complete":true},
            {"profile":"maximum-cleaning","package_checks_complete":true},
            {"profile":strategy::AGGRESSIVE,"checks_complete":true,"sheets":[{"status":"same"}],"failures":[]}
        ]});
        finish_without_capture(&mut record);
        assert_eq!(record["status"], "unchanged");
        assert_eq!(record["visual_coverage"], "all profiles");
    }
}
