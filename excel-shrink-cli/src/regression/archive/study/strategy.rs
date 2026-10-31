//! Experimental scheduling keeps aggressive screening distinct from full validation.
use super::*;

pub(super) const PRIMARY: &str = "aggressive_primary";
pub(super) const SECONDARY: &str = "secondary_full";
pub(super) const HASH_SCREEN: &str = "preparation_hash_screen";
pub(super) const AGGRESSIVE: &str = "too-aggressive-cleaning";

pub(super) fn pass(record: &Value) -> &str {
    record["comparison_pass"].as_str().unwrap_or("full")
}

pub(super) fn is_priority(record: &Value) -> bool {
    matches!(pass(record), PRIMARY | SECONDARY | HASH_SCREEN)
}

// A historical row-height result may have completed its own policy without
// comparing pixels on every selected sheet. Only full pixel checks or verified
// package identity satisfy this experiment's later independent validation.
pub(super) fn profile_complete(profile: &Value) -> bool {
    profile["package_checks_complete"] == true
        || (profile["checks_complete"] == true && profile["image_comparison"] == "complete")
}
pub(super) fn profiles(record: &Value) -> &'static [&'static str] {
    match pass(record) {
        HASH_SCREEN => match record["comparison_hash_profile"].as_str() {
            Some("default") => &["default"],
            Some("maximum-cleaning") => &["maximum-cleaning"],
            Some(AGGRESSIVE) => &[AGGRESSIVE],
            _ => &[],
        },
        PRIMARY => &[AGGRESSIVE],
        SECONDARY => {
            let aggressive_checked = record["results"].as_array().is_some_and(|profiles| {
                profiles
                    .iter()
                    .any(|profile| profile["profile"] == AGGRESSIVE && profile_complete(profile))
            });
            if aggressive_checked {
                &["default", "maximum-cleaning"]
            } else {
                &prepared::PROFILES
            }
        }
        _ => &prepared::PROFILES,
    }
}

pub(super) fn full_complete(record: &Value) -> bool {
    record["complete"] == true
        && matches!(
            record["status"].as_str(),
            Some("unchanged" | "differences" | "crc-identical" | "workbook-only")
        )
        && prepared::PROFILES.iter().all(|name| {
            record["results"].as_array().is_some_and(|profiles| {
                profiles
                    .iter()
                    .any(|profile| profile["profile"] == *name && profile_complete(profile))
            })
        })
}

pub(super) fn primary_terminal(record: &Value) -> bool {
    full_complete(record)
        || matches!(
            record["priority_validation"]["primary"]["status"].as_str(),
            Some("complete" | "failed")
        )
}

pub(super) fn needs_pass(record: &Value, comparison_pass: &str) -> bool {
    match comparison_pass {
        HASH_SCREEN => {
            record["complete"] != true
                || record["status"] == "skipped"
                || record["comparison_hash_profile"] != record["last_hash_screen_profile"]
                || record["last_hash_capture_policy"] != crate::regression::office_capture::ResidentCapture::CAPTURE_POLICY
        }
        PRIMARY => !primary_terminal(record),
        SECONDARY => !full_complete(record),
        _ => true,
    }
}

pub(super) fn finish(record: &mut Value) {
    if pass(record) == HASH_SCREEN {
        if record["status"] != "skipped" {
            record["last_hash_screen_profile"] = record["comparison_hash_profile"].clone();
            record["last_hash_capture_policy"] = json!(crate::regression::office_capture::ResidentCapture::CAPTURE_POLICY);
            record["checks_complete"] = json!(false);
            record["validation_scope"] = json!("selected profile and captured sheet regions with fixed 1920-pixel right margin; content beyond the margin is not verified");
        }
    } else if pass(record) == PRIMARY {
        let aggressive = record["results"].as_array().and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["profile"] == AGGRESSIVE)
        });
        let checked = record["status"] != "skipped" && aggressive.is_some_and(profile_complete);
        let changed = aggressive.is_some_and(|profile| {
            profile["failures"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty())
                || profile["sheets"]
                    .as_array()
                    .is_some_and(|sheets| sheets.iter().any(|sheet| sheet["status"] == "changed"))
        });
        let visual_status = aggressive
            .map(|profile| profile["status"].clone())
            .unwrap_or(Value::Null);
        if !checked
            && record["error"]
                .as_str()
                .is_some_and(|error| error.contains("CAPTURE_CHANGED"))
        {
            for profile in record["results"].as_array_mut().into_iter().flatten() {
                if profile["profile"] == AGGRESSIVE {
                    profile["visual_diagnosis"] = json!({"status":"unstable","category":"unstable",
                        "reason":"Capture changed between hash sweep and evidence replay.","scope":"attempted capture regions"});
                }
            }
        }
        record["priority_validation"]["version"] = json!(1);
        record["priority_validation"]["primary"] = json!({
            "status": if checked { "complete" } else { "failed" },
            "profile": AGGRESSIVE, "scope": "aggressive_profile",
            "differences_found": changed, "result": visual_status,
            "error": record["error"], "run_id": record["last_execution"]["run_id"],
            "elapsed_seconds": record["comparing_elapsed_seconds"],
            "completed_unix_ms": provenance::now_ms()
        });
        record["priority_validation"]["secondary"]["status"] = json!("pending");
        record["checks_complete"] = json!(false);
        record["complete"] = json!(false);
        record["status"] = json!(if !checked {
            "screening-failed"
        } else if changed {
            "screening-differences"
        } else {
            "screened"
        });
    } else if pass(record) == SECONDARY {
        let complete = record["status"] != "skipped"
            && prepared::PROFILES.iter().all(|name| {
                record["results"].as_array().is_some_and(|profiles| {
                    profiles
                        .iter()
                        .any(|profile| profile["profile"] == *name && profile_complete(profile))
                })
            });
        record["priority_validation"]["secondary"] = json!({
            "status": if complete { "complete" } else { "failed" },
            "scope": "all_profiles", "completed_unix_ms": provenance::now_ms(),
            "elapsed_seconds": record["comparing_elapsed_seconds"],
            "run_id": record["last_execution"]["run_id"]
        });
        record["checks_complete"] = json!(complete);
        if !complete && record["status"] != "skipped" {
            record["complete"] = json!(false);
            record["status"] = json!("validation-incomplete");
            if record["error"].is_null() {
                record["error"] = json!(
                    "Full profile validation remains incomplete; resume to retry the unchecked profiles."
                );
            }
        }
    }
}

pub(super) fn default_regression_candidate(record: &Value) -> bool {
    record["results"].as_array().is_some_and(|profiles| {
        profiles.iter().any(|profile| {
            profile["profile"] == "default"
                && profile["sheets"]
                    .as_array()
                    .is_some_and(|sheets| sheets.iter().any(|sheet| sheet["status"] == "changed"))
                && !matches!(
                    profile["visual_diagnosis"]["category"].as_str(),
                    Some("unstable" | "recalculation" | "identical")
                )
        })
    })
}

pub(super) fn diagnosis_categories(record: &Value) -> Vec<String> {
    let mut categories = std::collections::BTreeSet::new();
    for profile in record["results"].as_array().into_iter().flatten() {
        if let Some(category) = profile["visual_diagnosis"]["category"].as_str() {
            categories.insert(category.to_owned());
        }
    }
    categories.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_screen_reruns_when_the_selected_profile_changes() {
        let mut record = json!({
            "comparison_pass": HASH_SCREEN,
            "comparison_hash_profile": "default",
            "complete": true,
            "status": "screened-equal",
        });
        assert!(needs_pass(&record, HASH_SCREEN));
        finish(&mut record);
        assert!(!needs_pass(&record, HASH_SCREEN));
        assert_eq!(record["checks_complete"], false);
        record["last_hash_capture_policy"] = json!("tall-sheet-v1");
        assert!(needs_pass(&record, HASH_SCREEN));
        finish(&mut record);
        record["comparison_hash_profile"] = json!("maximum-cleaning");
        assert!(needs_pass(&record, HASH_SCREEN));
    }

    #[test]
    fn screening_never_claims_all_profiles_were_checked() {
        let mut record = json!({"comparison_pass":PRIMARY,"complete":true,"status":"unchanged",
            "results":[{"profile":AGGRESSIVE,"checks_complete":true,"image_comparison":"complete","status":"visually unchanged","failures":[]}]});
        finish(&mut record);
        assert_eq!(record["complete"], false);
        assert_eq!(record["checks_complete"], false);
        assert_eq!(record["status"], "screened");
        assert!(primary_terminal(&record));
        assert!(!needs_pass(&record, PRIMARY));
        assert!(needs_pass(&record, SECONDARY));
    }

    #[test]
    fn secondary_requires_the_primary_result_and_both_other_profiles() {
        let mut record = json!({"comparison_pass":SECONDARY,"results":[
            {"profile":"default","checks_complete":true,"image_comparison":"complete"},
            {"profile":"maximum-cleaning","checks_complete":true,"image_comparison":"complete"}]});
        finish(&mut record);
        assert_eq!(
            record["priority_validation"]["secondary"]["status"],
            "failed"
        );
        record["results"].as_array_mut().unwrap().push(
            json!({"profile":AGGRESSIVE,"checks_complete":true,"image_comparison":"complete"}),
        );
        finish(&mut record);
        assert_eq!(
            record["priority_validation"]["secondary"]["status"],
            "complete"
        );
    }

    #[test]
    fn targeted_followups_are_not_full_comparisons() {
        let mut record = json!({"comparison_pass":SECONDARY,"complete":true,"status":"differences","results":[
            {"profile":AGGRESSIVE,"checks_complete":true,"image_comparison":"complete"},
            {"profile":"default","targeted_checks_complete":true,"checks_complete":false},
            {"profile":"maximum-cleaning","targeted_checks_complete":true,"checks_complete":false}]});
        assert!(!full_complete(&record));
        finish(&mut record);
        assert_eq!(record["checks_complete"], false);
    }
    #[test]
    fn missing_mandatory_evidence_cannot_certify_primary() {
        let mut record = json!({"comparison_pass":PRIMARY,"status":"skipped","error":"preserve image evidence failed",
            "results":[{"profile":AGGRESSIVE,"checks_complete":true,"image_comparison":"complete"}]});
        finish(&mut record);
        assert_eq!(record["priority_validation"]["primary"]["status"], "failed");
        assert_eq!(record["complete"], false);
    }
    #[test]
    fn historical_row_height_policy_is_not_a_full_pixel_comparison() {
        let mut record = json!({"complete":true,"status":"differences","results":[]});
        for name in prepared::PROFILES {
            record["results"].as_array_mut().unwrap().push(json!({"profile":name,"checks_complete":true,
                "image_comparison":"complete on sheets with equal row heights; changed rows captured as evidence"}));
        }
        assert!(!full_complete(&record));
        assert!(needs_pass(&record, PRIMARY));
        for profile in record["results"].as_array_mut().unwrap() {
            profile["image_comparison"] = json!("complete");
        }
        assert!(full_complete(&record));
        assert!(!needs_pass(&record, PRIMARY));
    }
}
