//! Reuse images only within one workbook comparison, for identical input bytes.
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct VisualComparisons {
    entries: Vec<Entry>,
}

struct Entry {
    original_sha: String,
    shrunk_sha: String,
    profile: String,
    all_sheets: bool,
    covered: BTreeSet<usize>,
    visual: Value,
}

impl VisualComparisons {
    pub(super) fn remember(
        &mut self,
        completed: &Value,
        visual: Value,
        original_sha: String,
        shrunk_sha: String,
        all_sheets: bool,
    ) {
        if completed["checks_complete"] != true
            || !matches!(
                completed["status"].as_str(),
                Some("visual differences" | "visually unchanged" | "cell value differences")
            )
        {
            return;
        }
        let Some(profile) = visual["profile"].as_str() else {
            return;
        };
        let Some(items) = visual["sheets"].as_array() else {
            return;
        };
        let mut covered = BTreeSet::new();
        for sheet in items {
            let Some(index) = sheet["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
            else {
                return;
            };
            if !matches!(sheet["status"].as_str(), Some("same" | "changed"))
                || !covered.insert(index)
            {
                return;
            }
        }
        if covered.is_empty() {
            return;
        }
        self.entries.push(Entry {
            original_sha,
            shrunk_sha,
            profile: profile.into(),
            all_sheets,
            covered,
            visual,
        });
    }

    pub(super) fn find(
        &self,
        original_sha: &str,
        shrunk_sha: &str,
        selected: &Option<BTreeSet<usize>>,
        profile: &str,
    ) -> Option<Value> {
        let entry = self.entries.iter().find(|entry| {
            entry.original_sha == original_sha
                && entry.shrunk_sha == shrunk_sha
                && match selected {
                    Some(sheets) => !sheets.is_empty() && sheets.is_subset(&entry.covered),
                    None => entry.all_sheets,
                }
        })?;
        let mut visual = entry.visual.clone();
        let items = visual["sheets"].as_array_mut()?;
        if let Some(selected) = selected {
            items.retain(|sheet| selected.contains(&(sheet["index"].as_u64().unwrap() as usize)));
        }
        let checked = items.len();
        let unchanged = items
            .iter()
            .filter(|sheet| sheet["status"] == "same")
            .count();
        let heights = items.iter().any(|sheet| {
            sheet["row_heights"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        });
        visual["profile"] = json!(profile);
        visual["requested_sheets"] = json!(selected);
        visual["checked_sheets"] = json!(checked);
        visual["unchanged_sheets"] = json!(unchanged);
        visual["image_comparison"] = json!(if heights {
            "complete on sheets with equal row heights; changed rows captured as evidence"
        } else {
            "complete"
        });
        visual["comparison_reused_from"] = json!(entry.profile);
        Some(visual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Value {
        json!({"profile":"too-aggressive-cleaning","sheets":[
            {"index":1,"status":"same","row_heights":[],"image_differences":[]},
            {"index":2,"status":"changed","row_heights":[],"image_differences":[{"original_image":"original.png","shrunk_image":"shrunk.png"}]},
            {"index":3,"status":"changed","row_heights":[{"before":12,"after":15}],"image_differences":[]}]})
    }

    fn complete() -> Value {
        json!({"checks_complete":true,"status":"visual differences"})
    }

    #[test]
    fn identical_inputs_reuse_only_the_requested_sheets_and_keep_evidence_paths() {
        let mut cache = VisualComparisons::default();
        cache.remember(
            &complete(),
            source(),
            "original".into(),
            "shrunk".into(),
            true,
        );
        let selected = Some(BTreeSet::from([1, 2]));
        let reused = cache
            .find("original", "shrunk", &selected, "maximum-cleaning")
            .unwrap();
        assert_eq!(reused["profile"], "maximum-cleaning");
        assert_eq!(reused["comparison_reused_from"], "too-aggressive-cleaning");
        assert_eq!(reused["requested_sheets"], json!([1, 2]));
        assert_eq!(reused["checked_sheets"], 2);
        assert_eq!(reused["unchanged_sheets"], 1);
        assert_eq!(reused["image_comparison"], "complete");
        assert_eq!(
            reused["sheets"][1]["image_differences"][0]["shrunk_image"],
            "shrunk.png"
        );
        assert_eq!(
            cache.find("original", "shrunk", &None, "default").unwrap()["sheets"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn different_input_bytes_and_uncovered_sheets_never_reuse() {
        let mut cache = VisualComparisons::default();
        cache.remember(
            &complete(),
            source(),
            "original".into(),
            "shrunk".into(),
            false,
        );
        assert!(cache
            .find("other", "shrunk", &Some(BTreeSet::from([2])), "default")
            .is_none());
        assert!(cache
            .find("original", "other", &Some(BTreeSet::from([2])), "default")
            .is_none());
        assert!(cache
            .find("original", "shrunk", &Some(BTreeSet::from([4])), "default")
            .is_none());
        assert!(cache
            .find("original", "shrunk", &Some(BTreeSet::new()), "default")
            .is_none());
        assert!(cache.find("original", "shrunk", &None, "default").is_none());
    }

    #[test]
    fn failed_incomplete_or_malformed_results_never_enter_the_cache() {
        let mut cache = VisualComparisons::default();
        for result in [
            json!({"checks_complete":false,"status":"visual differences"}),
            json!({"checks_complete":true,"status":"technical error"}),
            json!({}),
        ] {
            cache.remember(&result, source(), "original".into(), "shrunk".into(), true);
        }
        let mut duplicate = source();
        duplicate["sheets"][1]["index"] = json!(1);
        cache.remember(
            &complete(),
            duplicate,
            "original".into(),
            "shrunk".into(),
            true,
        );
        assert!(cache.find("original", "shrunk", &None, "default").is_none());
    }

    #[test]
    fn selecting_only_unchanged_sheets_does_not_inherit_other_differences() {
        let mut cache = VisualComparisons::default();
        cache.remember(
            &complete(),
            source(),
            "original".into(),
            "shrunk".into(),
            true,
        );
        let result = cache
            .find("original", "shrunk", &Some(BTreeSet::from([1])), "default")
            .unwrap();
        assert_eq!(result["checked_sheets"], 1);
        assert_eq!(result["unchanged_sheets"], 1);
        assert!(result["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|sheet| sheet["status"] == "same"));
    }
}
