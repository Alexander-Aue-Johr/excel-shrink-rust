//! Presentation categories require positive evidence; absent diagnostics stay unresolved.
use super::{items, text};
use serde_json::Value;

pub(super) fn profile(file: &Value, result: &Value, sheet: &Value) -> &'static str {
    if !items(sheet, "row_heights").is_empty() && items(sheet, "image_differences").is_empty() {
        return "height";
    }
    if result["profile"] == "default" {
        return "other";
    }
    let diagnosis = &file["whitespace_diagnosis"];
    if diagnosis["version"] == 1 && diagnosis["status"] == "complete" {
        let classification = items(diagnosis, "profiles")
            .iter()
            .find(|p| p["profile"] == result["profile"])
            .and_then(|p| {
                items(p, "sheets")
                    .iter()
                    .find(|s| s["index"] == sheet["index"])
            });
        if classification.is_some_and(|s| {
            s["category"] == "whitespace_explained"
                && s["checks_complete"] == true
                && s["original_evidence_reproduced"] == true
        }) {
            return "whitespace";
        }
    }
    "other"
}

pub(super) fn sheet(file: &Value, index: u64) -> &'static str {
    let categories: Vec<_> = items(file, "results")
        .iter()
        .filter_map(|result| {
            items(result, "sheets")
                .iter()
                .find(|s| s["index"] == index && s["status"] == "changed")
                .map(|s| profile(file, result, s))
        })
        .collect();
    if categories.is_empty() || categories.contains(&"other") {
        "other"
    } else if categories.contains(&"height") {
        "height"
    } else {
        "whitespace"
    }
}

pub(super) fn file(file: &Value) -> &'static str {
    if items(file, "results").iter().any(|r| {
        r["status"] == "cell value differences"
            || r["value_differences"].as_u64().unwrap_or(0) > 0
            || r["checks_complete"] == false && !items(r, "failures").is_empty()
    }) {
        return "other";
    }
    let categories: Vec<_> = items(file, "results")
        .iter()
        .flat_map(|p| items(p, "sheets"))
        .filter(|s| s["status"] == "changed")
        .filter_map(|s| s["index"].as_u64())
        .map(|i| sheet(file, i))
        .collect();
    if categories.is_empty() || categories.contains(&"other") {
        "other"
    } else if categories.contains(&"height") {
        "height"
    } else {
        "whitespace"
    }
}

pub(super) fn rank(category: &str) -> u8 {
    match category {
        "whitespace" => 2,
        "height" => 1,
        _ => 0,
    }
}

pub(super) fn label(category: &str) -> &'static str {
    match category {
        "whitespace" => "Visual differences explained by whitespace removal",
        "height" => "Row heights changed – remaining visual differences not fully checked",
        _ => "Other or unexplained differences",
    }
}

pub(super) fn diagnosis_label(file: &Value) -> &str {
    match text(&file["whitespace_diagnosis"], "status") {
        "complete" => "Whitespace diagnosis completed; attribution is recorded per sheet and profile.",
        "failed" => "Whitespace diagnosis failed; the cause remains unexplained.",
        "running" => "Whitespace diagnosis is running; the cause is not yet established.",
        _ => "Whitespace cause not yet investigated. Existing differences have not been attributed to whitespace removal.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_mixed_and_failed_diagnoses_cannot_hide_a_workbook_in_whitespace() {
        let mut book = json!({"results":[{"profile":"maximum-cleaning","sheets":[
            {"index":1,"status":"changed","image_differences":[{}]}]}]});
        assert_eq!(file(&book), "other");
        book["whitespace_diagnosis"] = json!({"version":1,"status":"complete","profiles":[
            {"profile":"maximum-cleaning","sheets":[{"index":1,"category":"whitespace_explained",
            "checks_complete":true,"original_evidence_reproduced":true}]}]});
        assert_eq!(file(&book), "whitespace");
        book["results"]
            .as_array_mut()
            .unwrap()
            .push(json!({"profile":"default","sheets":[
            {"index":2,"status":"changed","image_differences":[{}]}]}));
        assert_eq!(file(&book), "other");
        assert_eq!(sheet(&book, 1), "whitespace");
        book["results"].as_array_mut().unwrap().pop();
        book["whitespace_diagnosis"]["status"] = json!("failed");
        assert_eq!(file(&book), "other");
        book["results"][0]["sheets"][0]["row_heights"] = json!([{"before":12,"after":14}]);
        assert_eq!(file(&book), "other");
        book["results"][0]["sheets"][0]["image_differences"] = json!([]);
        assert_eq!(file(&book), "height");
        book["results"][0]["value_differences"] = json!(1);
        assert_eq!(file(&book), "other");
    }
}
