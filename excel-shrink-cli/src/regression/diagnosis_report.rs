use super::visual_report::{
    anchor, escape_html as escape, permalink, render_differences, REPORT_END,
};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn write(output: &Path, report: &Value) -> Result<PathBuf> {
    fs::create_dir_all(output)?;
    let root = fs::canonicalize(output)?;
    let files = items(report, "workbooks");
    let refresh = if report["complete"] == false {
        "<meta http-equiv=\"refresh\" content=\"20\">"
    } else {
        ""
    };
    let mut html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">{refresh}<title>Excel Shrink | Visual diagnosis</title>
<style>{}</style></head><body><div class="layout"><nav class="sidebar"><div class="brand">EXCEL SHRINK</div>
<h2>Visual diagnosis</h2><a class="nav-link" href="#overview">Overview</a>
<input id="sheet-search" class="search" type="search" placeholder="Filter differences">"##,
        include_str!("visual_report.css")
    );
    for file in files {
        let id = file_id(file);
        let _ = write!(
            html,
            "<a class=\"nav-link\" href=\"#{id}\">{}</a>",
            escape(text(file, "name"))
        );
    }
    let complete = files.iter().filter(|f| f["complete"] == true).count();
    let _ = write!(html, "</nav><main class=\"main\"><header id=\"overview\" class=\"hero\"><div class=\"eyebrow\">EXCEL SHRINK</div><h1>Visual diagnosis</h1><p>{complete} / {} workbooks completed</p></header>", report["total_workbooks"].as_u64().unwrap_or(files.len() as u64));
    for failure in items(&report["aggregate"], "failures") {
        let _ = write!(
            html,
            "<p class=\"difference\">{}</p>",
            escape(failure.as_str().unwrap_or("Aggregate check failed"))
        );
    }
    for file in files {
        let id = file_id(file);
        let results = items(file, "results");
        let baseline = results.first().unwrap_or(&Value::Null);
        let equal = baseline["unchanged_sheets"].as_u64().unwrap_or_else(|| {
            items(baseline, "sheets")
                .iter()
                .filter(|s| s["status"] == "same")
                .count() as u64
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
        let _ = write!(html, "<section id=\"{id}\" class=\"workbook\"><header class=\"hero\"><h2>{}</h2>{}<p class=\"muted\">{}</p><p>{equal} unchanged worksheets · {changed} with differences</p><p>{}</p></header>",
            escape(text(file, "name")), permalink(&id), escape(text(file, "identity")),
            if file["complete"] == true { "Complete" } else { "In progress" });
        for result in results {
            let _ = write!(
                html,
                "<article class=\"difference\"><h3>{}</h3><p>{}</p>",
                escape(text(result, "profile")),
                escape(text(result, "status"))
            );
            if let Some(ratio) = result["reduction_ratio"].as_f64() {
                let _ = write!(html, "<p>Reduction: {:.2}%</p>", ratio * 100.0);
            }
            for failure in items(result, "failures") {
                let _ = write!(
                    html,
                    "<p>{}</p>",
                    escape(failure.as_str().unwrap_or("Unknown failure"))
                );
            }
            html.push_str("</article>");
        }
        for (index, name) in sheets {
            if !has_change(index) {
                continue;
            }
            let sheet_id = anchor("sheet", &format!("{id}:{index}:{name}"));
            let _ = write!(html, "<section id=\"{sheet_id}\" class=\"worksheet\" data-search=\"{} {}\"><div class=\"sheet-title\"><h2>Sheet {index:02} · {}</h2>{}</div>", escape(text(file,"name")), escape(name), escape(name), permalink(&sheet_id));
            for result in results {
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
                let profile = text(result, "profile");
                let _ = write!(
                    html,
                    "<article class=\"difference\"><h3>{}</h3><p>{}</p>",
                    escape(profile),
                    escape(text(sheet, "status"))
                );
                render_differences(
                    &mut html,
                    &root,
                    sheet,
                    &format!("{}:{index}:{name}:{profile}", text(file, "identity")),
                );
                html.push_str("</article>");
            }
            html.push_str("</section>");
        }
        html.push_str("</section>");
    }
    html.push_str(REPORT_END);
    let path = output.join("report.html");
    super::artifact_cache::atomic_write(&path, html.as_bytes())?;
    Ok(path)
}
fn sheet_result(profile: &Value, index: u64) -> Option<&Value> {
    items(profile, "sheets")
        .iter()
        .find(|sheet| sheet["index"].as_u64() == Some(index))
}

fn file_id(file: &Value) -> String {
    anchor("file", text(file, "identity"))
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn anchors_survive_append_and_order_changes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("diagnosis-report-{}", std::process::id()));
        let file = json!({"identity":"a/book.xlsx","name":"book.xlsx","complete":true,"results":[
            {"profile":"maximum-cleaning","status":"visual differences","sheets":[{"index":1,"name":"Sheet <1>","status":"changed","row_heights":[{"row":4,"before":12,"after":16}]}]},
            {"profile":"default","status":"visually unchanged","sheets":[{"index":1,"name":"Sheet <1>","status":"same"}]},
            {"profile":"keep-all-inactive-protection-cells","status":"technical error","requested_sheets":[1],"sheets":[],"failures":["test failure"]}]});
        let first = fs::read_to_string(write(&dir, &json!({"workbooks":[file]}))?)?;
        let second = fs::read_to_string(write(
            &dir,
            &json!({"workbooks":[{"identity":"b/book.xlsx","name":"book.xlsx"},file]}),
        )?)?;
        let ids = |s: &str| -> Vec<String> {
            regex::Regex::new("id=\"((?:file|difference)-[^\"]+)\"")
                .unwrap()
                .captures_iter(s)
                .map(|c| c[1].to_owned())
                .collect()
        };
        for id in ids(&first) {
            assert!(ids(&second).contains(&id));
        }
        assert!(first.contains("maximum-cleaning"));
        assert!(first.contains("default"));
        assert!(first.contains("visually unchanged"));
        assert!(first.contains("Sheet &lt;1&gt;"));
        assert!(first.contains("hashchange"));
        assert_eq!(first.matches("technical error").count(), 2);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
