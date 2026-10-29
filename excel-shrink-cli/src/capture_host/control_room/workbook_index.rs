//! A paged view of the study's durable preparation and comparison records.
use serde_json::{json, Value};

pub(super) const FILTERS: [&str; 23] = [
    "all",
    "crc-unchanged",
    "workbook-only",
    "namespace",
    "preparation-failed",
    "other-excluded",
    "needs-visual",
    "identical",
    "differences",
    "deferred",
    "recalc-drift",
    "screened",
    "secondary-pending",
    "default-investigate",
    "diagnosis-row_height",
    "diagnosis-local_whitespace",
    "diagnosis-local_hidden_content",
    "diagnosis-cross_sheet_hidden_content",
    "diagnosis-cross_sheet_whitespace",
    "diagnosis-recalculation",
    "diagnosis-combined",
    "diagnosis-unstable",
    "diagnosis-unresolved",
];

fn matches_filter(row: &Value, filter: &str) -> bool {
    match filter {
        "all" => true,
        "crc-unchanged" => row["preparation_disposition"] == "crc_unchanged",
        "workbook-only" => row["preparation_disposition"] == "workbook_only",
        "namespace" => row["preparation_disposition"] == "unsupported_namespace",
        "preparation-failed" => matches!(
            row["preparation_disposition"].as_str(),
            Some("shrink_failed" | "preparation_failed")
        ),
        "other-excluded" => {
            let disposition = row["preparation_disposition"].as_str();
            disposition.is_some_and(|value| {
                !matches!(
                    value,
                    "needs_visual"
                        | "crc_unchanged"
                        | "workbook_only"
                        | "unsupported_namespace"
                        | "shrink_failed"
                        | "preparation_failed"
                )
            })
        }
        "needs-visual" => row["preparation_disposition"] == "needs_visual",
        "identical" => row["visually_identical"] == true,
        "differences" => row["differences_found"] == true,
        "deferred" => row["has_capture_deferred"] == true,
        "recalc-drift" => row["has_recalc_identity_changed"] == true,
        "default-investigate" => row["default_regression_candidate"] == true,
        "screened" => row["priority_validation"]["primary"]["status"] == "complete",
        "secondary-pending" => row["priority_validation"].is_object() && row["priority_validation"]["secondary"]["status"] != "complete",
        value if value.starts_with("diagnosis-") => row["diagnosis_categories"].as_array()
            .is_some_and(|categories| categories.iter().any(|category| category.as_str() == value.strip_prefix("diagnosis-"))),
        _ => false,
    }
}

fn matches_search(row: &Value, search: &str) -> bool {
    if search.is_empty() {
        return true;
    }
    let search = search.to_lowercase();
    ["name", "corpus"]
        .into_iter()
        .filter_map(|field| row[field].as_str())
        .any(|value| value.to_lowercase().contains(&search))
}

pub(super) fn page(
    index: &Value,
    filter: &str,
    search: &str,
    offset: usize,
    limit: usize,
) -> Value {
    let rows = index["workbooks"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut counts = json!({});
    for category in FILTERS {
        counts[category] = json!(rows
            .iter()
            .filter(|row| matches_filter(row, category))
            .count());
    }
    let selected: Vec<_> = rows
        .iter()
        .filter(|row| matches_filter(row, filter) && matches_search(row, search))
        .collect();
    let total = selected.len();
    let page: Vec<_> = selected
        .into_iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect();
    json!({
        "version": index["version"],
        "run_id": index["run_id"],
        "visual_scope_frozen": index["visual_scope_frozen"],
        "counts": counts,
        "workbooks": page,
        "total": total,
        "offset": offset,
        "next_offset": offset.checked_add(limit).filter(|next| *next < total),
    })
}

pub(super) fn find<'a>(index: &'a Value, id: &str) -> Option<&'a Value> {
    index["workbooks"]
        .as_array()?
        .iter()
        .find(|row| row["id"] == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_and_screening_filters_do_not_claim_full_identity() {
        let index = json!({"workbooks":[
            {"id":"a","priority_validation":{"primary":{"status":"complete"},"secondary":{"status":"pending"}},
             "diagnosis_categories":["row_height","combined"],"visually_identical":false},
            {"id":"b","diagnosis_categories":["unstable"]},
            {"id":"c","priority_validation":{"primary":{"status":"failed"},"secondary":{"status":"failed"}}}
        ]});
        let result = page(&index, "screened", "", 0, 10);
        assert_eq!(result["total"], 1);
        assert_eq!(result["counts"]["secondary-pending"], 2);
        assert_eq!(result["counts"]["diagnosis-row_height"], 1);
        assert_eq!(result["counts"]["diagnosis-combined"], 1);
        assert_eq!(result["counts"]["diagnosis-unstable"], 1);
        assert_eq!(result["counts"]["identical"], 0);
    }
    #[test]
    fn categories_are_derived_from_all_records_before_paging() {
        let index = json!({"version":1,"visual_scope_frozen":true,"workbooks":[
            {"id":"a","name":"alpha.xlsx","corpus":"Data.gov","preparation_disposition":"crc_unchanged"},
            {"id":"b","name":"beta.xlsx","corpus":"Data.gov","preparation_disposition":"needs_visual","differences_found":true},
            {"id":"c","name":"gamma.xlsx","corpus":"Destatis","preparation_disposition":"needs_visual","has_capture_deferred":true},
            {"id":"d","name":"delta.xlsx","corpus":"Enron","preparation_disposition":"evidence_missing"},
            {"id":"e","name":"epsilon.xlsx","corpus":"Enron","preparation_disposition":"preparation_failed"},
        ]});
        let result = page(&index, "needs-visual", "", 0, 1);
        assert_eq!(result["counts"]["all"], 5);
        assert_eq!(result["counts"]["crc-unchanged"], 1);
        assert_eq!(result["counts"]["needs-visual"], 2);
        assert_eq!(result["counts"]["differences"], 1);
        assert_eq!(result["counts"]["other-excluded"], 1);
        assert_eq!(result["counts"]["preparation-failed"], 1);
        assert_eq!(result["total"], 2);
        assert_eq!(result["next_offset"], 1);
        assert_eq!(result["workbooks"][0]["id"], "b");
        assert_eq!(
            page(&index, "all", "DESTATIS", 0, 100)["workbooks"][0]["id"],
            "c"
        );
        assert!(find(&index, "missing").is_none());
    }
}
