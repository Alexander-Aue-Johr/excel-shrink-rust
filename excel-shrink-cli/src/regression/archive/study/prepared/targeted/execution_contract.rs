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
                            && decision["method"] == "explicit-rebuild"
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

fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
