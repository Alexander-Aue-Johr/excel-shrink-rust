mod tables;
mod workbook;

use super::visual_report::{anchor, escape_html as escape, permalink, REPORT_END};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn write(output: &Path, report: &Value) -> Result<PathBuf> {
    fs::create_dir_all(output)?;
    let root = fs::canonicalize(output)?;
    let corpora = collect_corpora(report);
    let files: Vec<_> = items(report, "workbooks").iter().collect();
    write_page(&root, "report.html", report, &files, &corpora, None)?;
    for corpus in corpora.values() {
        write_page(
            &root,
            &corpus.page(),
            report,
            &corpus.files,
            &corpora,
            Some(corpus),
        )?;
    }
    Ok(output.join("report.html"))
}

struct Corpus<'a> {
    id: String,
    name: String,
    total: Option<u64>,
    files: Vec<&'a Value>,
}

impl Corpus<'_> {
    fn page(&self) -> String {
        format!("{}.html", anchor("corpus", &self.id))
    }
}

fn collect_corpora(report: &Value) -> BTreeMap<String, Corpus<'_>> {
    let mut corpora = BTreeMap::new();
    for corpus in items(report, "corpora") {
        let id = text(corpus, "id").to_owned();
        corpora.insert(
            id.clone(),
            Corpus {
                id,
                name: text(corpus, "name").into(),
                total: corpus["total_workbooks"].as_u64(),
                files: Vec::new(),
            },
        );
    }
    for file in items(report, "workbooks") {
        let legacy_name = text(file, "name").split_once(" / ").map(|(name, _)| name);
        let id = file["corpus_id"]
            .as_str()
            .or(legacy_name)
            .unwrap_or("workbooks");
        let name = file["corpus_name"]
            .as_str()
            .or(legacy_name)
            .unwrap_or("Workbooks");
        corpora
            .entry(id.to_owned())
            .or_insert_with(|| Corpus {
                id: id.into(),
                name: name.into(),
                total: None,
                files: Vec::new(),
            })
            .files
            .push(file);
    }
    corpora
}

fn write_page(
    root: &Path,
    filename: &str,
    report: &Value,
    files: &[&Value],
    corpora: &BTreeMap<String, Corpus<'_>>,
    corpus: Option<&Corpus<'_>>,
) -> Result<()> {
    let refresh = if report["complete"] == false {
        "<meta http-equiv=\"refresh\" content=\"20\">"
    } else {
        ""
    };
    let title = corpus
        .map(|c| c.name.as_str())
        .unwrap_or("Visual diagnosis");
    let changed: Vec<_> = files
        .iter()
        .copied()
        .filter(|f| has_differences(f))
        .collect();
    let unchanged: Vec<_> = files.iter().copied().filter(|f| is_unchanged(f)).collect();
    let skipped: Vec<_> = files.iter().copied().filter(|f| is_skipped(f)).collect();
    let completed = files.iter().filter(|f| f["complete"] == true).count();
    let mut html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">{refresh}<title>Excel Shrink | {}</title>
<style>{}</style></head><body><div class="layout"><nav class="sidebar"><div class="brand">EXCEL SHRINK</div>
<h2>Visual diagnosis</h2><a class="nav-link" href="report.html">All corpora</a><a class="nav-link" href="#overview">Overview</a>"##,
        escape(title),
        include_str!("visual_report.css")
    );
    for item in corpora.values() {
        let active = if corpus.is_some_and(|c| c.id == item.id) {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(
            html,
            "<a class=\"nav-link\" href=\"{}\"{active}>{}</a>",
            item.page(),
            escape(&item.name)
        );
    }
    if !unchanged.is_empty() {
        html.push_str("<a class=\"nav-link\" href=\"#unchanged\">Unchanged workbooks</a>");
    }
    if !skipped.is_empty() {
        html.push_str("<a class=\"nav-link\" href=\"#skipped\">Skipped / errors</a>");
    }
    if !changed.is_empty() {
        html.push_str("<input id=\"sheet-search\" class=\"search\" type=\"search\" placeholder=\"Filter differences\" aria-label=\"Filter differences\">");
        for file in &changed {
            let _ = write!(
                html,
                "<a class=\"nav-link\" href=\"#{}\">{}</a>",
                file_id(file),
                escape(text(file, "name"))
            );
        }
    }
    let _ = write!(html, "</nav><main class=\"main\"><header id=\"overview\" class=\"hero\"><div class=\"eyebrow\">EXCEL SHRINK</div><h1>{}</h1><p>{completed} workbooks processed", escape(title));
    let total = corpus.and_then(|c| c.total).or_else(|| {
        if corpus.is_none() {
            report["total_workbooks"].as_u64()
        } else {
            None
        }
    });
    if let Some(total) = total {
        let _ = write!(html, " / {total} total");
    }
    let _ = write!(html, "</p><p>{}</p><div class=\"metrics\"><div class=\"metric\"><strong>{}</strong><span>Unchanged</span></div><div class=\"metric\"><strong>{}</strong><span>With differences</span></div><div class=\"metric\"><strong>{}</strong><span>Skipped / errors</span></div></div><p class=\"muted\">{} unchanged · {} with differences · {} skipped/errors</p></header>",
        if report["complete"] == true { "Run completed" } else { "Run not finished" }, unchanged.len(), changed.len(), skipped.len(), unchanged.len(), changed.len(), skipped.len());
    if report["limited"] == true {
        html.push_str("<p>Limited test run. Remaining archive entries have not been checked.</p>");
    }
    if corpus.is_none() && !corpora.is_empty() {
        corpus_cards(&mut html, corpora);
    } else if corpus.is_some() {
        html.push_str("<section class=\"hero\"><h2>Shrink measurements</h2>");
        tables::totals(&mut html, files);
        html.push_str("</section>");
    }
    for failure in items(&report["aggregate"], "failures") {
        let _ = write!(
            html,
            "<p class=\"difference\">{}</p>",
            escape(failure.as_str().unwrap_or("Aggregate check failed"))
        );
    }
    let waiting = files
        .iter()
        .filter(|f| matches!(f["status"].as_str(), Some("pending" | "prepared")))
        .count();
    if waiting > 0 {
        let _ = write!(
            html,
            "<p class=\"muted\">{waiting} workbooks awaiting checks</p>"
        );
    }
    for file in files.iter().filter(|f| {
        f["complete"] != true && !matches!(f["status"].as_str(), Some("pending" | "prepared"))
    }) {
        let _ = write!(
            html,
            "<p class=\"active-workbook\">In progress: {}</p>",
            escape(text(file, "name"))
        );
    }
    tables::unchanged(&mut html, &unchanged);
    tables::skipped(&mut html, &skipped);
    if !changed.is_empty() {
        html.push_str("<h2 class=\"section-heading\">Visual differences</h2>");
    }
    for file in changed {
        workbook::render(&mut html, root, file);
    }
    html.push_str(REPORT_END);
    super::artifact_cache::atomic_write(&root.join(filename), html.as_bytes())?;
    Ok(())
}

fn corpus_cards(html: &mut String, corpora: &BTreeMap<String, Corpus<'_>>) {
    html.push_str("<div class=\"corpus-grid\">");
    for corpus in corpora.values() {
        let unchanged = corpus.files.iter().filter(|f| is_unchanged(f)).count();
        let changed = corpus.files.iter().filter(|f| has_differences(f)).count();
        let skipped = corpus.files.iter().filter(|f| is_skipped(f)).count();
        let processed = corpus
            .files
            .iter()
            .filter(|f| f["complete"] == true)
            .count();
        let total = corpus.total.map(|n| format!(" / {n}")).unwrap_or_default();
        let _ = write!(html, "<article class=\"hero corpus-card\"><h2><a href=\"{}\">{}</a></h2><p>{processed}{total} workbooks processed</p><p>{unchanged} unchanged · {changed} with differences · {skipped} skipped/errors</p>", corpus.page(), escape(&corpus.name));
        tables::totals(html, &corpus.files);
        let _ = write!(
            html,
            "<a class=\"button\" href=\"{}\">Open corpus report</a></article>",
            corpus.page()
        );
    }
    html.push_str("</div>");
}

fn is_skipped(file: &Value) -> bool {
    matches!(file["status"].as_str(), Some("skipped" | "archive error"))
}
fn has_differences(file: &Value) -> bool {
    !is_skipped(file)
        && (file["status"] == "differences"
            || items(file, "results").iter().any(|r| {
                matches!(
                    r["status"].as_str(),
                    Some("visual differences" | "cell value differences")
                ) || items(r, "sheets").iter().any(|s| s["status"] == "changed")
            }))
}
fn is_unchanged(file: &Value) -> bool {
    file["complete"] == true
        && !is_skipped(file)
        && !has_differences(file)
        && (file["status"] == "unchanged"
            || items(file, "results")
                .iter()
                .any(|r| r["status"] == "visually unchanged"))
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
    fn unchanged_workbooks_are_collapsed_and_corpus_pages_keep_stable_links() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("corpus-report-{}", std::process::id()));
        let report = json!({"complete":true,"corpora":[
            {"id":"a","name":"Corpus A","total_workbooks":100},
            {"id":"b","name":"Corpus B","total_workbooks":20}],"workbooks":[
            {"identity":"old","name":"equal.xlsx","corpus_id":"a","complete":true,"previously_verified":true,"status":"unchanged","results":[
                {"profile":"maximum-cleaning","status":"visually unchanged","reduction_ratio":0.9}]},
            {"identity":"new","name":"changed.xlsx","corpus_id":"b","complete":true,"status":"differences","results":[
                {"profile":"default","status":"visual differences","shrink_seconds":1.25,"original_open_seconds":4,"shrunk_open_seconds":2,"original_memory_bytes":104857600,"shrunk_memory_bytes":52428800,"sheets":[
                    {"index":1,"name":"Sheet1","status":"changed","row_heights":[{"row":1,"before":12,"after":14}]}]}]}]});
        let html = fs::read_to_string(write(&dir, &report)?)?;
        assert!(html.contains("<details class=\"hero workbook-list\" id=\"unchanged\">"));
        assert!(html.contains("Previously verified"));
        assert!(html.contains("90.00%"));
        assert!(html.contains("1.25 s"));
        assert!(html.contains("100.0 MiB → 50.0 MiB"));
        assert_eq!(html.matches("class=\"workbook\"").count(), 1);
        let corpus_a = fs::read_to_string(dir.join(format!("{}.html", anchor("corpus", "a"))))?;
        assert!(corpus_a.contains("equal.xlsx"));
        assert!(!corpus_a.contains("changed.xlsx"));
        assert!(corpus_a.contains("1 workbooks processed / 100 total"));
        assert!(corpus_a.contains(&format!("id=\"{}\"", anchor("file", "old"))));
        let corpus_b = fs::read_to_string(dir.join(format!("{}.html", anchor("corpus", "b"))))?;
        assert!(corpus_b.contains(&format!("id=\"{}\"", anchor("file", "new"))));
        assert!(!corpus_b.contains("equal.xlsx"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn queued_workbooks_are_summarized_without_being_called_active() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("queued-report-{}", std::process::id()));
        let report = json!({"complete":false,"workbooks":[
            {"identity":"a","name":"pending.xlsx","status":"pending","complete":false},
            {"identity":"b","name":"prepared.xlsx","status":"prepared","complete":false},
            {"identity":"c","name":"active.xlsx","status":"comparing","complete":false}
        ]});
        let html = fs::read_to_string(write(&dir, &report)?)?;
        assert!(html.contains("2 workbooks awaiting checks"));
        assert_eq!(html.matches("class=\"active-workbook\"").count(), 1);
        assert!(html.contains("In progress: active.xlsx"));
        assert!(!html.contains("pending.xlsx"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn adaptive_report_counts_the_full_comparison_and_orders_visual_evidence() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("adaptive-report-{}", std::process::id()));
        let report = json!({"complete":true,"workbooks":[{
        "identity":"chart","name":"chart.xlsx","complete":true,"status":"differences","results":[
            {"profile":"default","status":"not compared","checks_complete":false,"sheets":[]},
            {"profile":"maximum-cleaning","status":"visually unchanged","checks_complete":true,
             "requested_sheets":[1],"checked_sheets":1,"unchanged_sheets":1,
             "sheets":[{"index":1,"name":"Chart","status":"same"}]},
            {"profile":"too-aggressive-cleaning","status":"visual differences","checks_complete":true,
             "requested_sheets":null,"checked_sheets":5,"unchanged_sheets":4,
             "sheets":[{"index":1,"name":"Chart","status":"changed","row_heights":[{"row":1,"before":12,"after":14}]}]}
        ]}]});
        let html = fs::read_to_string(write(&dir, &report)?)?;
        assert!(html.contains("4 unchanged worksheets · 1 with differences"));
        let worksheet = html.split("class=\"worksheet\"").nth(1).unwrap();
        assert!(
            worksheet.find("<h3>too-aggressive-cleaning</h3>").unwrap()
                < worksheet.find("<h3>maximum-cleaning</h3>").unwrap()
        );
        assert!(!worksheet.contains("<h3>default</h3>"));
        let metrics = html.split("<th>Profile / result</th>").nth(1).unwrap();
        assert!(
            metrics.find("<th scope=\"row\">default").unwrap()
                < metrics.find("<th scope=\"row\">maximum-cleaning").unwrap()
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn archive_report_distinguishes_skips_from_completed_checks() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("archive-report-{}", std::process::id()));
        let report = json!({"archive_run":true,"limited":true,"complete":true,"workbooks":[
            {"identity":"a","name":"valid.xlsx","complete":true,"status":"unchanged","results":[]},
            {"identity":"b","name":"broken.xlsx","complete":true,"status":"skipped","error":"Repair <required>","results":[]}
        ]});
        let html = fs::read_to_string(write(&dir, &report)?)?;
        assert!(html.contains("2 workbooks processed"));
        assert!(html.contains("1 unchanged · 0 with differences · 1 skipped/errors"));
        assert!(html.contains("Repair &lt;required&gt;"));
        assert!(html.contains("Remaining archive entries have not been checked"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

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
