//! Reviewer-facing machine attribution, separate from visual-cause categories.
use super::{escape, file_id, items, text};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub(super) fn render_summary(html: &mut String, report: &Value, files: &[&Value]) {
    let metadata = &report["measurement_provenance"];
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for &file in files {
        if file["complete"] != true {
            continue;
        }
        let machine = file["provenance"]["comparison"]["machine_id"]
            .as_str()
            .unwrap_or("unknown");
        groups.entry(machine.to_owned()).or_default().push(file);
    }
    if groups.is_empty() {
        return;
    }
    html.push_str("<section id=\"machines\"><h2>Measurement machines</h2><p>Preparation, primary comparison and whitespace diagnosis retain separate machine references. Legacy measurements remain unknown. Open-time speedups use paired before/after measurements from the same run and machine; raw timings and memory values remain in each workbook record. Hardware, Excel version, storage and background load can affect both speed and memory.</p>");
    for (id, files) in groups {
        let mut referenced_runs = BTreeSet::new();
        for file in &files {
            collect_run_ids(&file["provenance"], &mut referenced_runs);
        }
        let runs: Vec<_> = items(metadata, "runs")
            .iter()
            .filter(|run| {
                run["machine_id"] == id
                    && run["run_id"]
                        .as_str()
                        .is_some_and(|run| referenced_runs.contains(run))
            })
            .collect();
        let snapshots_used: BTreeSet<_> = runs
            .iter()
            .filter_map(|run| run["machine_manifest"].as_str())
            .collect();
        let snapshots: Vec<_> = items(metadata, "machines")
            .iter()
            .filter(|m| {
                m["metadata"]["machine_id"] == id
                    && (snapshots_used.is_empty()
                        || m["path"]
                            .as_str()
                            .is_some_and(|path| snapshots_used.contains(path)))
            })
            .collect();
        let machine = snapshots.first().map(|m| &m["metadata"]);
        let label = machine
            .and_then(|m| m["label"].as_str())
            .unwrap_or(if id == "unknown" {
                "Unknown historical machine"
            } else {
                &id
            });
        let _ = write!(
            html,
            "<details><summary>{} · {} completed workbooks</summary>",
            escape(label),
            files.len()
        );
        if let Some(machine) = machine {
            let cpus: Vec<_> = items(machine, "cpu")
                .iter()
                .filter_map(|cpu| cpu["model"].as_str())
                .collect();
            let logical: u64 = items(machine, "cpu")
                .iter()
                .filter_map(|cpu| cpu["logical_cores"].as_u64())
                .sum();
            let ram = machine["physical_memory_bytes"]
                .as_u64()
                .map(|n| format!("{:.1} GiB", n as f64 / 1073741824.0))
                .unwrap_or_else(|| "unknown RAM".into());
            let _ = write!(
                html,
                "<p>{} · {} · {} logical cores · {}<br>{} {} · Excel {} ({})</p>",
                escape(text(machine, "hostname")),
                escape(&cpus.join(" / ")),
                logical,
                escape(&ram),
                escape(text(&machine["os"], "name")),
                escape(text(&machine["os"], "version")),
                escape(machine["excel"]["version"].as_str().unwrap_or("unknown")),
                escape(
                    machine["excel"]["bitness"]
                        .as_str()
                        .unwrap_or("unknown bitness")
                )
            );
        } else {
            html.push_str("<p>Machine metadata was not recorded. These results are excluded from machine-specific speedup summaries.</p>");
        }
        let mut ratios: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for file in &files {
            for profile in items(file, "results") {
                if let Some(ratio) = paired_ratio(profile, &id) {
                    ratios
                        .entry(text(profile, "profile").to_owned())
                        .or_default()
                        .push(ratio);
                }
            }
        }
        if !ratios.is_empty() {
            html.push_str("<table><thead><tr><th>Profile</th><th>Paired workbooks</th><th>Median open-time speedup</th></tr></thead><tbody>");
            for (profile, mut values) in ratios {
                values.sort_by(f64::total_cmp);
                let n = values.len();
                let median = if n % 2 == 0 {
                    (values[n / 2 - 1] + values[n / 2]) / 2.0
                } else {
                    values[n / 2]
                };
                let _ = write!(
                    html,
                    "<tr><td>{}</td><td>{n}</td><td>{median:.3}×</td></tr>",
                    escape(&profile)
                );
            }
            html.push_str("</tbody></table>");
        }
        html.push_str("<p>Immutable manifests: ");
        for snapshot in snapshots {
            if let Some(path) = safe_metadata_link(snapshot["path"].as_str()) {
                let _ = write!(html, "<a href=\"{}\">hardware snapshot</a> ", escape(path));
            }
        }
        for run in runs {
            if let Some(run_id) = run["run_id"]
                .as_str()
                .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
            {
                let _ = write!(
                    html,
                    "<a href=\"metadata/runs/{}.json\">run {}</a> ",
                    escape(run_id),
                    escape(run_id)
                );
            }
        }
        html.push_str("</p><details><summary>Workbook assignments</summary><ul>");
        for file in files {
            let name = file["name"]
                .as_str()
                .or_else(|| file["identity"].as_str())
                .unwrap_or("Workbook");
            let prepare = file["provenance"]["preparation"]["machine_id"]
                .as_str()
                .unwrap_or("unknown");
            let diagnose = file["provenance"]["whitespace_diagnosis"]["machine_id"]
                .as_str()
                .unwrap_or("unknown");
            let _ = write!(
                html,
                "<li><a href=\"#{}\">{}</a> · preparation: {} · whitespace diagnosis: {}</li>",
                escape(&file_id(file)),
                escape(name),
                escape(machine_label(metadata, prepare)),
                escape(machine_label(metadata, diagnose))
            );
        }
        html.push_str("</ul></details></details>");
    }
    html.push_str("</section>");
}

fn collect_run_ids<'a>(value: &'a Value, ids: &mut BTreeSet<&'a str>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map.get("run_id").and_then(Value::as_str) {
                ids.insert(id);
            }
            for value in map.values() {
                collect_run_ids(value, ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_run_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn machine_label<'a>(metadata: &'a Value, id: &'a str) -> &'a str {
    items(metadata, "machines")
        .iter()
        .find(|m| m["metadata"]["machine_id"] == id)
        .and_then(|m| m["metadata"]["label"].as_str())
        .unwrap_or(id)
}

fn paired_ratio(profile: &Value, machine: &str) -> Option<f64> {
    let before = &profile["open_measurement_provenance"]["original"];
    let after = &profile["open_measurement_provenance"]["shrunk"];
    if machine == "unknown"
        || !before.is_object()
        || before != after
        || before["machine_id"] != machine
        || before["run_id"].as_str().is_none()
    {
        return None;
    }
    let original = profile["original_open_seconds"].as_f64()?;
    let shrunk = profile["shrunk_open_seconds"].as_f64()?;
    (original.is_finite() && shrunk.is_finite() && original > 0.0 && shrunk > 0.0)
        .then_some(original / shrunk)
}

fn safe_metadata_link(value: Option<&str>) -> Option<&str> {
    value.filter(|path| {
        path.starts_with("metadata/machines/")
            && path.ends_with(".json")
            && !path.contains("..")
            && !path.contains(['\\', ':', '\0'])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn machine_summary_excludes_legacy_and_cross_run_ratios() {
        let reference = json!({"machine_id":"tower","run_id":"same"});
        let report = json!({"workbooks":[
            {"identity":"known","name":"<script>","complete":true,"provenance":{"comparison":{"machine_id":"tower"}},
                "results":[{"profile":"default","original_open_seconds":4.0,"shrunk_open_seconds":2.0,
                    "open_measurement_provenance":{"original":reference,"shrunk":reference}},
                    {"profile":"maximum-cleaning","original_open_seconds":100.0,"shrunk_open_seconds":1.0,
                    "open_measurement_provenance":{"original":reference,"shrunk":{"machine_id":"tower","run_id":"other"}}}]},
            {"identity":"legacy","complete":true,"results":[{"profile":"default","original_open_seconds":99.0,"shrunk_open_seconds":1.0}]}]});
        let mut html = String::new();
        render_summary(
            &mut html,
            &report,
            &items(&report, "workbooks").iter().collect::<Vec<_>>(),
        );
        assert!(html.contains("2.000×"));
        assert!(!html.contains("100.000×"));
        assert!(!html.contains("99.000×"));
        assert!(html.contains("Unknown historical machine"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
