use super::{escape, file_id, items, permalink, text};
use serde_json::Value;
use std::fmt::Write as _;

pub(super) fn unchanged(html: &mut String, files: &[&Value]) {
    if files.is_empty() {
        return;
    }
    let _ = write!(html, "<details class=\"hero workbook-list\" id=\"unchanged\"><summary>{} visually unchanged workbooks</summary><p class=\"muted\">Reduction bars show the percentage of file size removed. — means the measurement was not recorded.</p><div class=\"table-scroll\"><table class=\"results-table\"><thead><tr><th>Workbook / profile</th><th>Reduction</th><th>Shrink</th><th>Open: original → shrunk</th><th>Excel RAM: original → shrunk</th></tr></thead><tbody>", files.len());
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
            "<tr id=\"{id}\"><th scope=\"row\">{} {}</th><td>{}</td></tr>",
            escape(text(file, "name")),
            permalink(&id),
            escape(text(file, "error"))
        );
    }
    html.push_str("</tbody></table></div></details>");
}

pub(super) fn profiles(html: &mut String, file: &Value) {
    html.push_str("<div class=\"table-scroll\"><table class=\"results-table\"><thead><tr><th>Profile / result</th><th>Reduction</th><th>Shrink</th><th>Open: original → shrunk</th><th>Excel RAM: original → shrunk</th></tr></thead><tbody>");
    profile_rows(html, file);
    html.push_str("</tbody></table></div>");
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
