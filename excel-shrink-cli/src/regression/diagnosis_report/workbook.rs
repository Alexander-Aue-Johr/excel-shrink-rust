use super::super::visual_report::{anchor, permalink, render_differences};
use super::{categories, escape, file_id, items, tables, text};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

pub(super) fn render(html: &mut String, root: &Path, file: &Value) {
    let id = file_id(file);
    let results = items(file, "results");
    let baseline = results.iter().find(|result| {
        result["requested_sheets"].is_null()
            && result["checks_complete"] != false
            && (result["checked_sheets"].is_number() || !items(result, "sheets").is_empty())
    });
    let equal = baseline.map(|result| {
        result["unchanged_sheets"].as_u64().unwrap_or_else(|| {
            items(result, "sheets")
                .iter()
                .filter(|s| s["status"] == "same")
                .count() as u64
        })
    });
    let mut sheets = BTreeMap::new();
    for result in results {
        for sheet in items(result, "sheets") {
            sheets
                .entry(sheet["index"].as_u64().unwrap_or(0))
                .or_insert(text(sheet, "name"));
        }
    }
    let has_change = |index| {
        results.iter().any(|result| {
            sheet_result(result, index).is_some_and(|sheet| sheet["status"] == "changed")
        })
    };
    let changed = sheets.keys().filter(|&&index| has_change(index)).count();
    let summary = equal
        .map(|count| format!("{count} unchanged worksheets · {changed} with differences"))
        .unwrap_or_else(|| format!("{changed} worksheets with differences"));
    let _ = write!(html, "<section id=\"{id}\" class=\"workbook\"><header class=\"hero\"><h2>{}</h2>{}<p>{summary}</p><p>{}</p>",
        escape(text(file, "name")), permalink(&id), if file["complete"] == true { "Complete" } else { "In progress" });
    tables::profiles(html, file);
    let diagnosis = &file["whitespace_diagnosis"];
    let status = diagnosis["status"].as_str().unwrap_or("not_investigated");
    let _ = write!(
        html,
        "<p class=\"muted\" data-whitespace-diagnosis=\"{}\" lang=\"de\">{}</p>",
        escape(&status.replace('_', "-")),
        categories::diagnosis_label(file)
    );
    if let Some(error) = diagnosis["error"].as_str() {
        let _ = write!(html, "<p class=\"difference\">{}</p>", escape(error));
    }
    html.push_str("</header>");
    if let Some(error) = file["error"].as_str() {
        let _ = write!(html, "<article class=\"difference\"><h3>File could not be fully checked</h3><p>{}</p></article>", escape(error));
    }
    for result in results {
        if items(result, "failures").is_empty() {
            continue;
        }
        let _ = write!(
            html,
            "<details class=\"difference\"><summary>{}: check details</summary>",
            escape(text(result, "profile"))
        );
        for failure in items(result, "failures") {
            let _ = write!(
                html,
                "<p>{}</p>",
                escape(failure.as_str().unwrap_or("Unknown failure"))
            );
        }
        html.push_str("</details>");
    }
    let mut comparison_order: Vec<_> = results.iter().collect();
    comparison_order.sort_by_key(|result| match text(result, "profile") {
        "too-aggressive-cleaning" => 0,
        "maximum-cleaning" => 1,
        "default" => 2,
        _ => 3,
    });
    let mut sheets: Vec<_> = sheets.into_iter().collect();
    sheets.sort_by_key(|(index, _)| (categories::rank(categories::sheet(file, *index)), *index));
    for (index, name) in sheets {
        if !has_change(index) {
            continue;
        }
        let sheet_id = anchor("sheet", &format!("{id}:{index}:{name}"));
        let category = categories::sheet(file, index);
        let open = if category == "other" { " open" } else { "" };
        let _ = write!(html, "<details id=\"{sheet_id}\" class=\"worksheet\" data-change-category=\"{category}\"{open} data-search=\"{} {}\"><summary class=\"sheet-title\"><h2>Sheet {index:02} · {}</h2>{}<span lang=\"de\">{}</span></summary>", escape(text(file,"name")), escape(name), escape(name), permalink(&sheet_id), categories::label(category));
        // Show residual differences from the matched controls before the full
        // comparisons, so mixed sheets expose their non-whitespace evidence first.
        for profile in items(diagnosis, "profiles") {
            if let Some(sheet) = items(&profile["control"], "sheets")
                .iter()
                .find(|s| s["index"] == index && s["status"] == "changed")
            {
                let category = if items(sheet, "row_heights").is_empty() {
                    "other"
                } else {
                    "height"
                };
                let open = if category == "other" { " open" } else { "" };
                let _ = write!(html, "<details class=\"difference\" data-change-category=\"{category}\"{open}><summary>{}: ohne Whitespace-Löschung – verbleibende Unterschiede</summary>", escape(text(profile,"profile")));
                render_differences(
                    html,
                    root,
                    sheet,
                    &format!(
                        "{}:{index}:{}:whitespace-control",
                        text(file, "identity"),
                        text(profile, "profile")
                    ),
                );
                html.push_str("</details>");
            }
        }
        comparison_order.sort_by_key(|result| {
            let sheet = sheet_result(result, index);
            (
                sheet
                    .map(|s| {
                        if s["status"] == "same" {
                            3
                        } else {
                            categories::rank(categories::profile(file, result, s))
                        }
                    })
                    .unwrap_or(0),
                match text(result, "profile") {
                    "too-aggressive-cleaning" => 0,
                    "maximum-cleaning" => 1,
                    _ => 2,
                },
            )
        });
        for result in &comparison_order {
            let sheet = sheet_result(result, index);
            if sheet.is_none()
                && !items(result, "requested_sheets")
                    .iter()
                    .any(|i| i.as_u64() == Some(index))
            {
                continue;
            }
            let missing = serde_json::json!({"status":text(result,"status")});
            let sheet = sheet.unwrap_or(&missing);
            if sheet["status"] == "same" {
                continue;
            }
            let profile = text(result, "profile");
            let category = categories::profile(file, result, sheet);
            let open = if category == "other" { " open" } else { "" };
            let _ = write!(
                html,
                "<details class=\"difference\" data-change-category=\"{category}\"{open}><summary><h3>{}</h3> – {}</summary><p>{}</p>",
                escape(profile),
                categories::label(category),
                escape(text(sheet, "status"))
            );
            render_differences(
                html,
                root,
                sheet,
                &format!("{}:{index}:{name}:{profile}", text(file, "identity")),
            );
            html.push_str("</details>");
        }
        html.push_str("</details>");
    }
    html.push_str("</section>");
}

fn sheet_result(profile: &Value, index: u64) -> Option<&Value> {
    items(profile, "sheets")
        .iter()
        .find(|sheet| sheet["index"].as_u64() == Some(index))
}
