//! Aggressive-first screening with targeted follow-up, followed by a separate full pass.
use super::super::strategy;
use super::*;
use crate::regression::office_capture::ResidentCapture;
use std::collections::{BTreeMap, BTreeSet};

fn screened_sheet_summaries(evidence: &Value) -> Vec<Value> {
    evidence["sheets"].as_array().into_iter().flatten()
        .filter(|sheet| sheet["original_sha256"] == sheet["variant_sha256"])
        .map(|sheet| json!({
            "index":sheet["sheet_index"], "name":sheet["sheet_name"],
            "status":if sheet["capture_coverage"]["omitted_tiles"].as_u64().unwrap_or(0) > 0 {
                "sampled_same"
            } else { "hash_screened_same" },
            "capture_coverage":sheet["capture_coverage"],
            "comparison_method":"stable PNG hashes; no pixel replay"
        })).collect()
}

fn preparation_screen_sheets(
    record: &Value,
    profile: &str,
    original: &Path,
    variant: &Path,
) -> Result<Option<BTreeSet<usize>>> {
    let screen = &record["preparation_hash_screen"];
    if screen["capture_policy"] != ResidentCapture::CAPTURE_POLICY
        || screen["version"] != 1
        || screen["profile"] != profile
        || screen["stable"] != true
        || screen["original_file_sha256"] != artifact_cache::hash_file(original)?
        || screen["variant_file_sha256"] != artifact_cache::hash_file(variant)?
    {
        return Ok(None);
    }
    let Some(first) = screen["first_sheets"].as_array() else {
        return Ok(None);
    };
    let Some(sheets) = screen["sheets"].as_array() else {
        return Ok(None);
    };
    if sheets.is_empty() || first != sheets {
        return Ok(None);
    }
    let digest = artifact_cache::sha256(&serde_json::to_vec(&json!({
        "first": first, "second": sheets,
    }))?);
    if screen["global_sha256"] != digest {
        return Ok(None);
    }
    let mut changed = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for sheet in sheets {
        let Some(index) = sheet["sheet_index"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
        else {
            return Ok(None);
        };
        let Some(tiles) = sheet["tiles"].as_array() else {
            return Ok(None);
        };
        if !seen.insert(index) || tiles.is_empty() || sheet["tile_count"] != tiles.len() {
            return Ok(None);
        }
        if sheet["original_sha256"]
            .as_str()
            .is_none_or(|hash| hash.len() != 64)
            || sheet["variant_sha256"]
                .as_str()
                .is_none_or(|hash| hash.len() != 64)
        {
            return Ok(None);
        }
        if tiles.iter().any(|tile| {
            tile["original_sha256"]
                .as_str()
                .is_none_or(|hash| hash.len() != 64)
                || tile["variant_sha256"]
                    .as_str()
                    .is_none_or(|hash| hash.len() != 64)
        }) {
            return Ok(None);
        }
        if sheet["original_sha256"] != sheet["variant_sha256"] {
            changed.insert(index);
        }
    }
    Ok(Some(changed))
}

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

fn followup_profiles(primary_pass: bool, screened_profile: &str) -> &'static [&'static str] {
    if primary_pass {
        &["default", "maximum-cleaning"]
    } else {
        match screened_profile {
            "maximum-cleaning" => &["default"],
            strategy::AGGRESSIVE => &["maximum-cleaning", "default"],
            _ => &[],
        }
    }
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
        json!("primary mismatch regions")
    } else {
        Value::Null
    };
    visual["coverage_scope"] = json!(if targeted {
        "primary mismatch regions only"
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
    let differences = record["results"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|profile| {
            changed(profile)
                || profile["failures"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
        });
    record["status"] = json!(if differences {
        "differences"
    } else {
        "unchanged"
    });
}

fn finish_without_capture(record: &mut Value) {
    // The secondary pass may reuse an aggressive difference even when every
    // newly selected profile is package-identical and needs no Excel client.
    set_comparison_status(record);
    if strategy::pass(record) == strategy::HASH_SCREEN && record["status"] == "unchanged" {
        record["status"] = json!("screened-equal");
    }
    record["capture_mode"] = json!("package-audit");
    record["visual_coverage"] = json!(if strategy::pass(record) == strategy::HASH_SCREEN {
        "selected profile package identity; no visual comparison"
    } else if strategy::pass(record) == strategy::PRIMARY {
        "aggressive profile plus targeted follow-up"
    } else {
        "all profiles"
    });
    record["whitespace_diagnosis"] = json!({"version":super::super::whitespace::VERSION,
        "status":"not_applicable","reason":"selected profiles require no visual capture"});
}

pub(super) fn compare(job: &MemberJob, record: &mut Value) -> Result<()> {
    let primary = strategy::pass(record) == strategy::PRIMARY;
    let hash_screen = strategy::pass(record) == strategy::HASH_SCREEN;
    let selected = strategy::profiles(record).to_vec();
    let active: Vec<_> = selected
        .iter()
        .copied()
        .filter(|name| {
            !profile_archive_reused(record, name) && !profile_workbook_only(record, name)
        })
        .collect();
    for name in PROFILES {
        if hash_screen && !selected.contains(&name) {
            continue;
        }
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
    let screen_sheets = if hash_screen {
        let name = active[0];
        let original = job.work.join("original.xlsx");
        let variant = job.work.join(format!("{name}.xlsx"));
        let selected_sheets = preparation_screen_sheets(record, name, &original, &variant)?;
        record["hash_screening"] = json!({
            "profile": name,
            "status": if selected_sheets.is_some() { "accepted" } else { "full_visual_fallback" },
            "reason": if selected_sheets.is_none() {
                "missing, unstable, or incompatible preparation hashes"
            } else {
                "stable preparation hashes and matching workbook identities"
            },
            "changed_sheets": selected_sheets,
        });
        selected_sheets
    } else {
        None
    };
    if hash_screen && screen_sheets.as_ref().is_some_and(BTreeSet::is_empty) {
        let name = active[0];
        let evidence = record["preparation_hash_screen"].clone();
        let target = profile_mut(record, name)?;
        target["status"] = json!("hash-screened-equal");
        target["checks_complete"] = json!(false);
        target["image_comparison"] = json!("preparation hash screening only");
        target["sheets"] = json!(screened_sheet_summaries(&evidence));
        target["preparation_hash_screen"] = evidence;
        record["status"] = json!("screened-equal");
        record["visual_coverage"] = json!("stable preparation hashes in selected ranges; see per-sheet coverage for omissions; no pixel replay");
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
    let additional_profiles = if primary || hash_screen {
        followup_profiles(primary, active[0]).len()
    } else {
        0
    };
    let mut session = ResidentCapture::start_prioritized(
        &job.work.join("original.xlsx"),
        &variants,
        &directory,
        screen_sheets.as_ref(),
        additional_profiles,
    )?;
    let mut captured = Vec::new();
    let mut primary_visual = None;
    for name in &active {
        let output = job.work.join("comparisons").join(name).join("visual");
        let visual = session.compare(name, &output)?;
        if *name == strategy::AGGRESSIVE || hash_screen {
            primary_visual = Some(visual.clone());
        }
        save_visual(
            record,
            name,
            visual,
            validations.remove(name).unwrap_or_default(),
            hash_screen && screen_sheets.is_some(),
        )?;
        if hash_screen && screen_sheets.is_some() {
            let screened = screened_sheet_summaries(&record["preparation_hash_screen"]);
            let target = profile_mut(record, name)?;
            let sheets = target["sheets"].as_array_mut().context("missing visual sheets")?;
            for summary in screened {
                if !sheets.iter().any(|sheet| sheet["index"] == summary["index"]) {
                    sheets.push(summary);
                }
            }
            target["requested_sheets"] = json!("preparation hash mismatch sheets");
            target["coverage_scope"] = json!("sheets with differing stable preparation hashes");
        }
        captured.push(*name);
        write_record(&job.record, record)?;
    }
    if primary || hash_screen {
        if let Some(visual) = primary_visual.as_ref().filter(|visual| changed(visual)) {
            for &name in followup_profiles(primary, active[0]) {
                if profile_archive_reused(record, name) {
                    mark_crc_identical(profile_mut(record, name)?);
                    continue;
                }
                if profile_workbook_only(record, name) {
                    mark_workbook_only(profile_mut(record, name)?);
                    continue;
                }
                // Follow-up profiles are targeted raw-input captures. Only the
                // old two-pass strategy schedules full recalc validation later.
                let failures = Vec::new();
                profile_mut(record, name)?["recalculation_validation"] = json!(if hash_screen {
                    "not_performed_in_targeted_screening"
                } else {
                    "deferred_to_full_pass"
                });
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
    if hash_screen && record["status"] == "unchanged" {
        record["status"] = json!("screened-equal");
    }
    record["capture_policy"] = json!(if hash_screen {
        "preparation-hash-screening-v1"
    } else {
        "aggressive-first-autonomous-sweep-v1"
    });
    record["visual_coverage"] = json!(if hash_screen && screen_sheets.is_some() {
        "stable preparation hash differences; selected sheets visually compared"
    } else if hash_screen {
        "selected profile visually compared after hash fallback"
    } else if primary {
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
mod screen_tests {
    use super::*;

    #[test]
    fn hash_screen_followup_profiles_match_selected_mode() {
        assert!(followup_profiles(false, "default").is_empty());
        assert_eq!(followup_profiles(false, "maximum-cleaning"), &["default"]);
        assert_eq!(
            followup_profiles(false, strategy::AGGRESSIVE),
            &["maximum-cleaning", "default"]
        );
        assert_eq!(
            followup_profiles(true, strategy::AGGRESSIVE),
            &["default", "maximum-cleaning"]
        );
    }

    #[test]
    fn stable_screen_selects_only_changed_sheets_and_rejects_changed_inputs() -> Result<()> {
        let root = super::super::super::tests::test_directory("hash-screen")?;
        let original = root.join("original.xlsx");
        let variant = root.join("default.xlsx");
        fs::write(&original, b"original")?;
        fs::write(&variant, b"variant")?;
        let sheets = json!([
            {"sheet_index":0,"tile_count":1,"tiles":[{"original_sha256":"a".repeat(64),"variant_sha256":"a".repeat(64)}],
                "original_sha256":"a".repeat(64),
                "variant_sha256":"a".repeat(64)},
            {"sheet_index":2,"tile_count":1,"tiles":[{"original_sha256":"b".repeat(64),"variant_sha256":"c".repeat(64)}],
                "original_sha256":"b".repeat(64),
                "variant_sha256":"c".repeat(64)}
        ]);
        let global_sha256 = artifact_cache::sha256(&serde_json::to_vec(&json!({
            "first":sheets,"second":sheets,
        }))?);
        let mut record = json!({"preparation_hash_screen":{
            "version":1,"capture_policy":ResidentCapture::CAPTURE_POLICY,"profile":"default","stable":true,
            "original_file_sha256":artifact_cache::hash_file(&original)?,
            "variant_file_sha256":artifact_cache::hash_file(&variant)?,
            "first_sheets":sheets,"sheets":sheets,"global_sha256":global_sha256,
        }});
        assert_eq!(
            preparation_screen_sheets(&record, "default", &original, &variant)?,
            Some(BTreeSet::from([2]))
        );
        let mut equal = record.clone();
        let sheet = &mut equal["preparation_hash_screen"]["sheets"][1];
        sheet["variant_sha256"] = sheet["original_sha256"].clone();
        sheet["tiles"][0]["variant_sha256"] = sheet["tiles"][0]["original_sha256"].clone();
        equal["preparation_hash_screen"]["first_sheets"] =
            equal["preparation_hash_screen"]["sheets"].clone();
        equal["preparation_hash_screen"]["global_sha256"] =
            json!(artifact_cache::sha256(&serde_json::to_vec(&json!({
                "first": equal["preparation_hash_screen"]["first_sheets"],
                "second": equal["preparation_hash_screen"]["sheets"],
            }))?));
        assert_eq!(
            preparation_screen_sheets(&equal, "default", &original, &variant)?,
            Some(BTreeSet::new())
        );
        record["preparation_hash_screen"]["stable"] = json!(false);
        assert!(preparation_screen_sheets(&record, "default", &original, &variant)?.is_none());
        record["preparation_hash_screen"]["stable"] = json!(true);
        fs::write(&variant, b"changed")?;
        assert!(preparation_screen_sheets(&record, "default", &original, &variant)?.is_none());
        Ok(())
    }

    #[test]
    fn hash_equal_sheet_summaries_preserve_omissions_without_claiming_pixel_checks() {
        let summaries = screened_sheet_summaries(&json!({"sheets":[
            {"sheet_index":1,"sheet_name":"CSV","original_sha256":"a","variant_sha256":"a",
                "capture_coverage":{"omitted_tiles":9996,"captured_tiles":4}},
            {"sheet_index":2,"original_sha256":"b","variant_sha256":"c"},
            {"sheet_index":3,"original_sha256":"d","variant_sha256":"d"}
        ]}));
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0]["status"], "sampled_same");
        assert_eq!(summaries[0]["capture_coverage"]["omitted_tiles"], 9996);
        assert_eq!(summaries[1]["status"], "hash_screened_same");
        assert_eq!(summaries[0]["comparison_method"], "stable PNG hashes; no pixel replay");
    }
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
