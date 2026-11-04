//! Version-three reports may omit states only with an explicit, recognized proof.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;

/// Require every physically observed pair and reject unsupported omissions.
/// Disabled Check Performance lanes are valid only when explicitly requested.
pub(super) fn expected_pairs(
    report: &Value,
    skip_performance: bool,
) -> Result<BTreeSet<(&'static str, &'static str)>> {
    if !skip_performance {
        let outcome = report["check_performance"]
            .as_str()
            .context("Missing native Check Performance outcome")?;
        ensure!(
            matches!(outcome, "optimization_confirmed" | "no_optimization_needed")
                && report["check_performance_evidence"]["native_result"]["status"] == outcome,
            "Check Performance has no independently confirmed native outcome"
        );
        let cache = &report["check_performance_cache"];
        if cache["status"] == "hit" {
            ensure!(
                cache["policy"] == "check-performance-artifacts-v1"
                    && valid_hash(&cache["key"])
                    && cache["historical"]["native_result"]
                        == report["check_performance_evidence"]["native_result"]
                    && cache["measurements_are_historical"] == true,
                "Cached Check Performance lacks generation provenance"
            );
            for name in [
                "original_saved",
                "before_optimization",
                "after_optimization",
            ] {
                ensure!(
                    valid_hash(&cache["historical"]["artifacts"][name]["sha256"]),
                    "Cached Check Performance lacks an artifact checksum"
                );
            }
        }
    }
    if report["execution_strategy"] == "paired-inplace-rebuild-v1" {
        return paired_rebuild_pairs(report, skip_performance);
    }
    if report["execution_strategy"] == "default-inplace-rebuild-v1" {
        return inplace_pairs(report, skip_performance);
    }
    let identity = &report["input_identity"];
    let identical = identity["status"] == "identical";
    ensure!(
        identical || identity["status"] == "different",
        "Input identity was not checked"
    );
    if identical {
        ensure!(
            valid_hash(&identity["sha256"]) && identity["bytes"].as_u64().is_some(),
            "Invalid input identity proof"
        );
    }
    let decisions = report["state_decisions"]
        .as_object()
        .context("Missing state decisions")?;
    ensure!(decisions.len() == 6, "Incomplete state decisions");
    let mut observed = BTreeSet::new();
    for id in ["O", "SD", "CP", "OR", "SDR", "CPR"] {
        let decision = &report["state_decisions"][id];
        if skip_performance && matches!(id, "CP" | "CPR") {
            ensure!(
                decision["status"] == "disabled",
                "Disabled Check Performance state was not declared"
            );
            continue;
        }
        match id {
            "O" | "CP" => {
                ensure!(
                    decision["status"] == "observed" && decision["method"] == "open-save",
                    "Missing required original or independent Check Performance observation"
                );
                observed.insert(id);
            }
            "SD" => {
                if decision["status"] == "skipped_identical_input" {
                    ensure!(
                        identical && decision["source"] == "O",
                        "Default was omitted without byte identity"
                    );
                } else {
                    ensure!(
                        decision["status"] == "observed" && decision["method"] == "open-save",
                        "Missing Default observation"
                    );
                    observed.insert(id);
                }
            }
            _ => {
                let parent = match id {
                    "OR" => "O",
                    "SDR" => "SD",
                    _ => "CP",
                };
                if id == "SDR" && decision["status"] == "skipped_identical_input" {
                    ensure!(
                        identical
                            && decision["source"] == "O"
                            && report["state_decisions"]["SD"]["status"]
                                == "skipped_identical_input",
                        "Default rebuild was omitted without its input-identity exclusion"
                    );
                    continue;
                }
                ensure!(
                    observed.contains(parent),
                    "Rebuild decision has no observed parent"
                );
                let check = &report["rebuild_checks"][id];
                ensure!(
                    check["policy"] == "static-workbook-rebuild-v1"
                        && valid_hash(&check["source_sha256"]),
                    "Missing rebuild eligibility proof"
                );
                let blockers = check["blockers"]
                    .as_array()
                    .context("Missing rebuild blockers")?;
                let eligible = check["eligible_to_skip"]
                    .as_bool()
                    .context("Missing rebuild decision")?;
                if decision["status"] == "skipped_no_rebuild" {
                    ensure!(
                        decision["source"] == parent
                            && eligible
                            && blockers.is_empty()
                            && check["checked_worksheets"].as_u64().is_some_and(|n| n > 0),
                        "Unjustified rebuild exclusion"
                    );
                } else {
                    ensure!(
                        decision["status"] == "observed"
                            && (decision["method"] == "explicit-rebuild"
                                || (id == "OR" || id == "SDR")
                                    && decision["method"] == "cached-rebuild"
                                    && report["control_artifacts_cache"][id]["status"] == "hit"
                                    && report["control_artifacts_cache"][id]
                                        ["current_parent_sha256"]
                                        == check["source_sha256"]
                                    && report["control_artifacts_cache"][id]["historical"]
                                        ["identity"]["policy"]
                                        == "excel-control-artifacts-v1"
                                    && report["control_artifacts_cache"][id]["historical"]
                                        ["identity"]["lane"]
                                        == id
                                    && valid_hash(
                                        &report["control_artifacts_cache"][id]["historical"]
                                            ["artifacts"]["before_rebuild"]["sha256"]
                                    )
                                    && valid_hash(
                                        &report["control_artifacts_cache"][id]["historical"]
                                            ["artifacts"]["after_rebuild"]["sha256"]
                                    )
                                || id == "CPR"
                                    && decision["method"] == "cached-rebuild"
                                    && report["check_performance_cache"]["rebuild_reused"] == true
                                    && report["check_performance_cache"]["status"] == "hit"
                                    && valid_hash(
                                        &report["check_performance_cache"]["historical"]
                                            ["artifacts"]["after_rebuild"]["sha256"]
                                    ))
                            && !eligible
                            && !blockers.is_empty(),
                        "Missing required rebuild observation"
                    );
                    observed.insert(id);
                }
            }
        }
    }
    Ok([
        ("O", "SD"),
        ("O", "SDR"),
        ("SD", "SDR"),
        ("O", "OR"),
        ("OR", "SDR"),
        ("O", "CP"),
        ("O", "CPR"),
        ("CP", "CPR"),
        ("OR", "CPR"),
    ]
    .into_iter()
    .filter(|(a, b)| observed.contains(a) && observed.contains(b))
    .collect())
}

/// The frozen SD pre-state has phase-specific coverage; no SD-before/SD-after claim is made.
fn inplace_pairs(
    report: &Value,
    skip_performance: bool,
) -> Result<BTreeSet<(&'static str, &'static str)>> {
    let decisions = &report["state_decisions"];
    ensure!(
        decisions.as_object().is_some_and(|d| d.len() == 6),
        "Incomplete in-place decisions"
    );
    ensure!(
        decisions["O"]["status"] == "observed" && decisions["O"]["method"] == "open-save",
        "Missing retained Original observation"
    );
    ensure!(
        decisions["OR"]["status"] == "disabled" && decisions["CPR"]["status"] == "disabled",
        "Unexpected Original or CP rebuild in screening strategy"
    );
    let mut pairs = BTreeSet::new();
    if decisions["SD"]["status"] == "skipped_identical_input" {
        ensure!(
            report["input_identity"]["status"] == "identical"
                && valid_hash(&report["input_identity"]["sha256"])
                && report["input_identity"]["bytes"].as_u64().is_some()
                && decisions["SD"]["source"] == "O"
                && decisions["SDR"]["status"] == "skipped_identical_input"
                && decisions["SDR"]["source"] == "O",
            "Unproven identical SD exclusion"
        );
    } else {
        ensure!(
            decisions["SD"]["status"] == "observed"
                && decisions["SD"]["method"] == "open-save"
                && decisions["SDR"]["status"] == "observed"
                && decisions["SDR"]["method"] == "inplace-rebuild",
            "Missing SD before/after observation"
        );
        let pid = report["excel_pids"]["SD"]
            .as_u64()
            .context("Missing SD process identity")?;
        ensure!(
            pid > 0 && report["excel_pids"]["SDR"].as_u64() == Some(pid),
            "SD rebuild did not retain its original Excel process"
        );
        pairs.extend([("O", "SD"), ("O", "SDR")]);
    }
    if skip_performance {
        ensure!(
            decisions["CP"]["status"] == "disabled",
            "Unexpected Check Performance state"
        );
    } else {
        ensure!(
            decisions["CP"]["status"] == "observed" && decisions["CP"]["method"] == "open-save",
            "Missing optional Check Performance observation"
        );
        pairs.insert(("O", "CP"));
    }
    Ok(pairs)
}
/// Require both sides rebuilt in their original resident processes before accepting the fast screen.
fn paired_rebuild_pairs(
    report: &Value,
    skip_performance: bool,
) -> Result<BTreeSet<(&'static str, &'static str)>> {
    let decisions = &report["state_decisions"];
    ensure!(decisions.as_object().is_some_and(|d| d.len() == 6), "Incomplete paired rebuild decisions");
    let mut pairs = BTreeSet::new();
    require_resident_rebuild(report, "O", "OR")?;
    if decisions["SD"]["status"] == "skipped_identical_input" {
        ensure!(report["input_identity"]["status"] == "identical"
            && valid_hash(&report["input_identity"]["sha256"])
            && report["input_identity"]["bytes"].as_u64().is_some()
            && decisions["SD"]["source"] == "O"
            && decisions["SDR"]["status"] == "skipped_identical_input"
            && decisions["SDR"]["source"] == "O", "Unproven identical Default omission");
    } else {
        require_resident_rebuild(report, "SD", "SDR")?;
        pairs.insert(("OR", "SDR"));
    }
    if skip_performance {
        ensure!(decisions["CP"]["status"] == "disabled"
            && decisions["CPR"]["status"] == "disabled", "Unexpected Check Performance state");
    } else {
        require_resident_rebuild(report, "CP", "CPR")?;
        pairs.insert(("OR", "CPR"));
    }
    Ok(pairs)
}

fn require_resident_rebuild(report: &Value, source: &str, target: &str) -> Result<()> {
    let decisions = &report["state_decisions"];
    ensure!(decisions[source]["status"] == "observed"
        && decisions[source]["method"] == "open-save"
        && decisions[target]["status"] == "observed"
        && decisions[target]["method"] == "inplace-rebuild",
        "Missing observed rebuild: {source}/{target}");
    let pid = report["excel_pids"][source].as_u64().filter(|pid| *pid > 0)
        .context("Missing resident process identity")?;
    ensure!(report["excel_pids"][target].as_u64() == Some(pid),
        "Rebuild process changed: {source}/{target}");
    Ok(())
}

fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inplace_report() -> Value {
        json!({
            "execution_strategy":"default-inplace-rebuild-v1",
            "input_identity":{"status":"different"},
            "state_decisions":{
                "O":{"status":"observed","method":"open-save"},
                "SD":{"status":"observed","method":"open-save"},
                "SDR":{"status":"observed","method":"inplace-rebuild"},
                "OR":{"status":"disabled"},"CP":{"status":"disabled"},"CPR":{"status":"disabled"}
            },
            "excel_pids":{"O":10,"SD":20,"SDR":20}
        })
    }

    #[test]
    fn paired_screen_requires_both_resident_rebuilds() -> Result<()> {
        let mut report = inplace_report();
        report["execution_strategy"] = json!("paired-inplace-rebuild-v1");
        assert!(expected_pairs(&report, true).is_err());
        report["state_decisions"]["OR"] = json!({"status":"observed","method":"inplace-rebuild"});
        report["excel_pids"]["OR"] = json!(10);
        assert_eq!(expected_pairs(&report, true)?, BTreeSet::from([("OR", "SDR")]));
        report["excel_pids"]["OR"] = json!(11);
        assert!(expected_pairs(&report, true).is_err());
        report["excel_pids"]["OR"] = json!(10);
        report["state_decisions"]["CP"] = json!({"status":"observed","method":"open-save"});
        report["excel_pids"]["CP"] = json!(30);
        report["check_performance"] = json!("optimization_confirmed");
        report["check_performance_evidence"] = json!({"native_result":{"status":"optimization_confirmed"}});
        assert!(expected_pairs(&report, false).is_err());
        report["state_decisions"]["CPR"] = json!({"status":"observed","method":"inplace-rebuild"});
        report["excel_pids"]["CPR"] = json!(30);
        assert_eq!(expected_pairs(&report, false)?, BTreeSet::from([("OR", "SDR"), ("OR", "CPR")]));
        Ok(())
    }

    #[test]
    fn inplace_screen_requires_same_sd_process_and_no_original_rebuild() {
        let mut report = inplace_report();
        assert_eq!(
            expected_pairs(&report, true).unwrap(),
            BTreeSet::from([("O", "SD"), ("O", "SDR")])
        );
        report["excel_pids"]["SDR"] = json!(21);
        assert!(expected_pairs(&report, true).is_err());
        report["excel_pids"]["SDR"] = json!(20);
        report["state_decisions"]["OR"]["status"] = json!("observed");
        assert!(expected_pairs(&report, true).is_err());
    }

    #[test]
    fn optional_performance_still_requires_positive_native_confirmation() {
        let mut report = inplace_report();
        report["state_decisions"]["CP"] = json!({"status":"observed","method":"open-save"});
        assert!(expected_pairs(&report, false).is_err());
        report["check_performance"] = json!("optimization_confirmed");
        report["check_performance_evidence"] =
            json!({"native_result":{"status":"optimization_confirmed"}});
        assert_eq!(expected_pairs(&report, false).unwrap().len(), 3);
    }
    fn reduced() -> Value {
        let mut report = json!({
            "check_performance": "no_optimization_needed",
            "check_performance_evidence": {
                "native_result": {"status": "no_optimization_needed"}
            },
            "input_identity": {
                "status": "identical", "bytes": 100, "sha256": "a".repeat(64)
            },
            "state_decisions": {
                "O": {"status": "observed", "method": "open-save"},
                "CP": {"status": "observed", "method": "open-save"},
                "SD": {"status": "skipped_identical_input", "source": "O"},
                "SDR": {"status": "skipped_identical_input", "source": "O"},
                "OR": {"status": "skipped_no_rebuild", "source": "O"},
                "CPR": {"status": "skipped_no_rebuild", "source": "CP"}
            }
        });
        for id in ["OR", "CPR"] {
            report["rebuild_checks"][id] = json!({
                "policy": "static-workbook-rebuild-v1",
                "source_sha256": "b".repeat(64),
                "eligible_to_skip": true,
                "checked_worksheets": 1,
                "blockers": []
            });
        }
        report
    }

    #[test]
    fn identical_default_never_removes_independent_check_performance() -> Result<()> {
        let report = reduced();
        assert_eq!(expected_pairs(&report, false)?, [("O", "CP")].into());
        let mut broken = report.clone();
        broken["state_decisions"]["CP"]["status"] = json!("disabled");
        assert!(expected_pairs(&broken, false).is_err());
        let mut broken = report.clone();
        broken["input_identity"]["status"] = json!("different");
        assert!(expected_pairs(&broken, false).is_err());
        let mut broken = report.clone();
        broken["rebuild_checks"]["CPR"]["blockers"] = json!(["formula"]);
        assert!(expected_pairs(&broken, false).is_err());
        let mut broken = report.clone();
        broken["check_performance_evidence"]["native_result"]["status"] = json!("pending");
        assert!(expected_pairs(&broken, false).is_err());
        Ok(())
    }

    #[test]
    fn cached_original_rebuild_requires_bound_control_provenance() -> Result<()> {
        let mut report = reduced();
        report["state_decisions"]["OR"] = json!({"status":"observed","method":"cached-rebuild"});
        report["rebuild_checks"]["OR"]["eligible_to_skip"] = json!(false);
        report["rebuild_checks"]["OR"]["blockers"] = json!(["worksheet formula"]);
        assert!(expected_pairs(&report, false).is_err());
        report["control_artifacts_cache"]["OR"] = json!({"status":"hit",
            "current_parent_sha256":"b".repeat(64),
            "historical":{"identity":{"policy":"excel-control-artifacts-v1","lane":"OR"},
                "artifacts":{"before_rebuild":{"sha256":"c".repeat(64)},
                    "after_rebuild":{"sha256":"d".repeat(64)}}}});
        assert!(expected_pairs(&report, false).is_ok());
        report["control_artifacts_cache"]["OR"]["current_parent_sha256"] = json!("e".repeat(64));
        assert!(expected_pairs(&report, false).is_err());
        Ok(())
    }

    #[test]
    fn cached_rebuild_requires_independent_generation_provenance() -> Result<()> {
        let mut report = reduced();
        report["state_decisions"]["CPR"] = json!({"status":"observed","method":"cached-rebuild"});
        report["rebuild_checks"]["CPR"]["eligible_to_skip"] = json!(false);
        report["rebuild_checks"]["CPR"]["blockers"] = json!(["formula"]);
        report["check_performance_cache"] = json!({
            "status":"hit", "policy":"check-performance-artifacts-v1",
            "key":"c".repeat(64), "rebuild_reused":true,
            "measurements_are_historical":true,
            "historical":{"native_result":{"status":"no_optimization_needed"}}
        });
        for name in [
            "original_saved",
            "before_optimization",
            "after_optimization",
            "after_rebuild",
        ] {
            report["check_performance_cache"]["historical"]["artifacts"][name] =
                json!({"sha256":"d".repeat(64)});
        }
        assert_eq!(expected_pairs(&report, false)?.len(), 3);
        report["check_performance_cache"]["rebuild_reused"] = json!(false);
        assert!(expected_pairs(&report, false).is_err());
        report["check_performance_cache"]["rebuild_reused"] = json!(true);
        report["check_performance_cache"]["historical"]["native_result"]["status"] =
            json!("pending");
        assert!(expected_pairs(&report, false).is_err());
        Ok(())
    }
}
