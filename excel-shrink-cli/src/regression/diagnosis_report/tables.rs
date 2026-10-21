use super::{escape, file_id, items, permalink, text};
use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn unchanged(html: &mut String, files: &[&Value]) {
    if files.is_empty() {
        return;
    }
    let _ = write!(html, "<details class=\"hero workbook-list\" id=\"unchanged\"><summary>{} visually unchanged workbooks</summary><p class=\"muted\">Reduction bars show the percentage of file size removed. — means the measurement was not recorded. Excel opening uses a fresh process with manual calculation; RAM is its working set immediately after opening.</p><div class=\"table-scroll\"><table class=\"results-table\"><thead><tr><th>Workbook / profile</th><th>Reduction</th><th>Shrink</th><th>Open: original → shrunk</th><th>Excel RAM: original → shrunk</th></tr></thead><tbody>", files.len());
    for file in files {
        let id = file_id(file);
        let _ = write!(
            html,
            "<tr id=\"{id}\"><th colspan=\"5\" scope=\"rowgroup\">{} {}</th></tr>",
            escape(text(file, "name")),
            permalink(&id)
        );
        if file["previously_verified"] == true {
            html.push_str("<tr><td colspan=\"5\" class=\"muted\">Previously verified; missing measurements are shown as —.</td></tr>");
        }
        if file["conversion"].is_object() {
            html.push_str("<tr><td colspan=\"5\" class=\"muted\">Converted from XLS; comparisons and reduction measurements use the converted XLSX baseline.</td></tr>");
        }
        profile_rows(html, file);
    }
    html.push_str("</tbody></table></div></details>");
}

pub(super) fn skipped(html: &mut String, files: &[&Value]) {
    if files.is_empty() {
        return;
    }
    let _ = write!(html, "<details class=\"hero workbook-list\" id=\"skipped\"><summary>{} skipped files / errors</summary><div class=\"table-scroll\"><table class=\"results-table\"><thead><tr><th>Workbook</th><th>Reason</th></tr></thead><tbody>", files.len());
    for file in files {
        let id = file_id(file);
        let _ = write!(
            html,
            "<tr id=\"{id}\"><th scope=\"row\">{} {}</th><td><p>{}</p>",
            escape(text(file, "name")),
            permalink(&id),
            escape(text(file, "error"))
        );
        for result in items(file, "results") {
            let failures = items(result, "failures");
            if failures.is_empty() {
                continue;
            }
            let _ = write!(
                html,
                "<details><summary>{}: check details</summary><ul>",
                escape(text(result, "profile"))
            );
            for failure in failures {
                let _ = write!(
                    html,
                    "<li>{}</li>",
                    escape(failure.as_str().unwrap_or("Unknown failure"))
                );
            }
            html.push_str("</ul></details>");
        }
        html.push_str("</td></tr>");
    }
    html.push_str("</tbody></table></div></details>");
}

pub(super) fn profiles(html: &mut String, file: &Value) {
    if file["conversion"].is_object() {
        html.push_str("<p class=\"muted\">Converted from XLS; comparisons and reduction measurements use the converted XLSX baseline.</p>");
    }
    html.push_str("<div class=\"table-scroll\"><table class=\"results-table\"><thead><tr><th>Profile / result</th><th>Reduction</th><th>Shrink</th><th>Open: original → shrunk</th><th>Excel RAM: original → shrunk</th></tr></thead><tbody>");
    profile_rows(html, file);
    html.push_str("</tbody></table></div><p class=\"muted\">Open time: Workbooks.Open in a fresh Excel process with manual calculation. RAM: working set immediately after opening.</p>");
}

pub(super) fn totals(html: &mut String, files: &[&Value]) {
    html.push_str("<table class=\"results-table\"><thead><tr><th>Profile</th><th>Size removed</th><th>Shrink time</th></tr></thead><tbody>");
    for profile in ["default", "maximum-cleaning", "too-aggressive-cleaning"] {
        let results: Vec<_> = files
            .iter()
            .flat_map(|f| items(f, "results"))
            .filter(|r| r["profile"] == profile)
            .collect();
        let sizes: Vec<_> = results
            .iter()
            .filter_map(|r| Some((r["original_bytes"].as_u64()?, r["shrunk_bytes"].as_u64()?)))
            .collect();
        let before: f64 = sizes.iter().map(|(n, _)| *n as f64).sum();
        let after: f64 = sizes.iter().map(|(_, n)| *n as f64).sum();
        let durations: Vec<_> = results
            .iter()
            .filter_map(|r| r["shrink_seconds"].as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
            .collect();
        let saving = if before > 0.0 {
            format!(
                "{:.2}%<span class=\"profile-result muted\">{} measured files</span>",
                (1.0 - after / before) * 100.0,
                sizes.len()
            )
        } else {
            "—".into()
        };
        let elapsed = if durations.is_empty() {
            "—".into()
        } else {
            format!(
                "{:.2} s<span class=\"profile-result muted\">{} measured files</span>",
                durations.iter().sum::<f64>(),
                durations.len()
            )
        };
        let _ = write!(
            html,
            "<tr><th scope=\"row\">{profile}</th><td>{saving}</td><td>{elapsed}</td></tr>"
        );
    }
    html.push_str("</tbody></table>");
}

fn profile_rows(html: &mut String, file: &Value) {
    let results = items(file, "results");
    if results.is_empty() {
        html.push_str("<tr><td>—</td><td>—</td><td>—</td><td>— → —</td><td>— → —</td></tr>");
    }
    for result in results {
        let _ = write!(html, "<tr><th scope=\"row\">{}<span class=\"profile-result muted\">{}</span></th><td>{}</td><td>{}</td><td>{} → {}</td><td>{} → {}</td></tr>",
            escape(text(result, "profile")), escape(text(result, "status")), reduction(result),
            seconds(&result["shrink_seconds"]), seconds(&result["original_open_seconds"]), seconds(&result["shrunk_open_seconds"]),
            memory(&result["original_memory_bytes"]), memory(&result["shrunk_memory_bytes"]));
    }
}

fn reduction(result: &Value) -> String {
    let ratio = result["reduction_ratio"].as_f64().or_else(|| {
        let before = result["original_bytes"].as_f64()?;
        let after = result["shrunk_bytes"].as_f64()?;
        (before > 0.0).then_some(1.0 - after / before)
    });
    let Some(ratio) = ratio.filter(|value| value.is_finite()) else {
        return "—".into();
    };
    let percentage = ratio * 100.0;
    let tone = if ratio < 0.0 { "increased" } else { "reduced" };
    format!("<span class=\"reduction {tone}\"><strong>{percentage:.2}%</strong><span class=\"reduction-track\" aria-hidden=\"true\"><span style=\"width:{:.2}%\"></span></span></span>", percentage.abs().min(100.0))
}

fn seconds(value: &Value) -> String {
    value
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|seconds| format!("{seconds:.2} s"))
        .unwrap_or_else(|| "—".into())
}

fn memory(value: &Value) -> String {
    value
        .as_u64()
        .map(|bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0))
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skipped_workbooks_disclose_the_profile_failure() {
        let skipped_file = json!({"identity":"pivot","name":"pivot.xlsx",
        "error":"One or more comparisons could not be completed.","results":[
            {"profile":"default","failures":[]},
            {"profile":"too-aggressive-cleaning","failures":["/pair/heights: HTTP 400 <unexpected heights>"]}
        ]});
        let mut html = String::new();
        skipped(&mut html, &[&skipped_file]);
        assert!(html.contains("One or more comparisons could not be completed."));
        assert!(html.contains("<summary>too-aggressive-cleaning: check details</summary>"));
        assert!(html.contains("/pair/heights: HTTP 400 &lt;unexpected heights&gt;"));
        assert!(!html.contains("<summary>default"));
    }

    #[test]
    fn corpus_savings_are_weighted_by_bytes_and_missing_timings_are_not_zero() {
        let measured = json!({"results":[
            {"profile":"default","original_bytes":1000,"shrunk_bytes":100,"shrink_seconds":2.5}]});
        let older = json!({"results":[
            {"profile":"default","original_bytes":9000,"shrunk_bytes":9000}]});
        let mut html = String::new();
        totals(&mut html, &[&measured, &older]);
        assert!(html.contains("9.00%<span class=\"profile-result muted\">2 measured files"));
        assert!(html.contains("2.50 s<span class=\"profile-result muted\">1 measured files"));
        assert!(!html.contains("45.00%"));
    }
}
