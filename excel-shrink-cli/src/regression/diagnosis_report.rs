mod categories;
mod machines;
mod tables;
mod workbook;

use super::visual_report::{anchor, escape_html as escape, permalink, REPORT_END};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

// Missing legacy metadata is an unknown cause, never evidence of a whitespace effect.
pub(super) fn ensure_whitespace_diagnosis(record: &mut Value) {
    if record
        .get("whitespace_diagnosis")
        .is_none_or(Value::is_null)
    {
        record["whitespace_diagnosis"] = serde_json::json!({"status": "not_investigated"});
    }
}

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
    let mut changed: Vec<_> = files
        .iter()
        .copied()
        .filter(|f| has_differences(f))
        .collect();
    changed.sort_by_key(|file| {
        (
            categories::rank(categories::file(file)),
            !items(file, "results")
                .iter()
                .any(|r| r["profile"] == "default" && r["status"] == "visual differences"),
        )
    });
    let unchanged: Vec<_> = files.iter().copied().filter(|f| is_unchanged(f)).collect();
    let screened: Vec<_> = files.iter().copied().filter(|f| is_screened(f)).collect();
    let skipped: Vec<_> = files.iter().copied().filter(|f| is_skipped(f)).collect();
    let partial: Vec<_> = files
        .iter()
        .copied()
        .filter(|file| !is_skipped(file) && !has_differences(file) && has_omitted_captures(file))
        .collect();
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
    if !screened.is_empty() {
        html.push_str("<a class=\"nav-link\" href=\"#screened\">Hash screened</a>");
    }
    if !skipped.is_empty() {
        html.push_str("<a class=\"nav-link\" href=\"#skipped\">Skipped / errors</a>");
    }
    if !partial.is_empty() {
        html.push_str("<a class=\"nav-link\" href=\"#partial-captures\">Shortened tall sheets</a>");
    }
    if completed > 0 {
        html.push_str("<a class=\"nav-link\" href=\"#machines\">Measurement machines</a>");
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
    let _ = write!(html, "</p><p>{}</p><div class=\"metrics\"><div class=\"metric\"><strong>{}</strong><span>Unchanged</span></div><div class=\"metric\"><strong>{}</strong><span>With differences</span></div><div class=\"metric\"><strong>{}</strong><span>Hash screened</span></div><div class=\"metric\"><strong>{}</strong><span>Skipped / errors</span></div></div><p class=\"muted\">{} unchanged · {} with differences · {} hash screened · {} skipped/errors</p></header>",
        if report["complete"] == true { "Run completed" } else { "Run not finished" }, unchanged.len(), changed.len(), screened.len(), skipped.len(), unchanged.len(), changed.len(), screened.len(), skipped.len());
    package_summary(&mut html, &unchanged);
    if !partial.is_empty() {
        let _ = write!(html, "<p>{} workbooks without differences in the selected captures; tall-sheet ranges were omitted and remain visually unchecked.</p>", partial.len());
    }
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
    machines::render_summary(&mut html, report, files);
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
    if !screened.is_empty() {
        html.push_str("<h2 id=\"screened\" class=\"section-heading\">Hash screened</h2><p class=\"muted\">These workbooks completed the selected screening profile. Matching preparation hashes are not a complete visual or cell-value validation; only sheets with differing stable hashes received a pixel comparison.</p>");
        for file in &screened {
            workbook::render(&mut html, root, file);
        }
    }
    tables::skipped(&mut html, &skipped);
    if !changed.is_empty() {
        html.push_str("<h2 class=\"section-heading\">Visual differences</h2>");
        html.push_str(r#"<section class="hero" aria-label="Collapse difference categories"><p>Each workbook stays together. Unexplained differences appear first. Collapse categories:</p><label><input type="checkbox" data-collapse-category="whitespace" checked> Whitespace effects</label> <label><input type="checkbox" data-collapse-category="height" checked> Row-height findings</label> <label><input type="checkbox" data-collapse-category="other"> Other / unexplained differences</label> <button type="button" class="button" data-expand-categories>Expand all</button><p class="muted">Collapsing sections does not change results or counts. Complete visual comparison remains unresolved where row heights changed.</p></section>"#);
    }
    let mut previous_category = "";
    for file in changed {
        let category = categories::file(file);
        if category != previous_category {
            let _ = write!(
                html,
                "<h2 class=\"section-heading\" lang=\"de\">{}</h2>",
                categories::label(category)
            );
            previous_category = category;
        }
        workbook::render(&mut html, root, file);
    }
    if !partial.is_empty() {
        html.push_str("<h2 id=\"partial-captures\" class=\"section-heading\">Shortened tall sheets — no differences in selected captures</h2>");
        for file in partial {
            workbook::render(&mut html, root, file);
        }
    }
    html.push_str(REPORT_END);
    super::artifact_cache::atomic_write(&root.join(filename), html.as_bytes())?;
    Ok(())
}

fn corpus_cards(html: &mut String, corpora: &BTreeMap<String, Corpus<'_>>) {
    html.push_str("<div class=\"corpus-grid\">");
    for corpus in corpora.values() {
        let unchanged = corpus.files.iter().filter(|f| is_unchanged(f)).count();
        let screened = corpus.files.iter().filter(|f| is_screened(f)).count();
        let changed = corpus.files.iter().filter(|f| has_differences(f)).count();
        let skipped = corpus.files.iter().filter(|f| is_skipped(f)).count();
        let partial = corpus
            .files
            .iter()
            .filter(|f| !is_skipped(f) && !has_differences(f) && has_omitted_captures(f))
            .count();
        let processed = corpus
            .files
            .iter()
            .filter(|f| f["complete"] == true)
            .count();
        let total = corpus.total.map(|n| format!(" / {n}")).unwrap_or_default();
        let _ = write!(html, "<article class=\"hero corpus-card\"><h2><a href=\"{}\">{}</a></h2><p>{processed}{total} workbooks processed</p><p>{unchanged} unchanged · {changed} with differences · {screened} hash screened · {skipped} skipped/errors</p>", corpus.page(), escape(&corpus.name));
        if partial > 0 {
            let _ = write!(html, "<p>{partial} without differences in selected captures; tall-sheet ranges omitted</p>");
        }
        package_summary(html, &corpus.files);
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
fn is_screened(file: &Value) -> bool {
    file["complete"] == true && file["status"] == "screened-equal"
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
        && !is_screened(file)
        && !is_skipped(file)
        && !has_differences(file)
        && !has_omitted_captures(file)
        && (file["status"] == "unchanged"
            || items(file, "results")
                .iter()
                .any(|r| r["status"] == "visually unchanged"))
}

fn has_omitted_captures(file: &Value) -> bool {
    items(file, "results")
        .iter()
        .flat_map(|result| items(result, "sheets"))
        .any(super::visual_report::has_omissions)
}

fn is_package_unchanged(file: &Value) -> bool {
    is_unchanged(file)
        && file["capture_mode"] == "package-audit"
        && file["package_identity"]["status"] == "equivalent"
}

fn package_summary(html: &mut String, files: &[&Value]) {
    let count = files
        .iter()
        .filter(|file| is_package_unchanged(file))
        .count();
    if count > 0 {
        let _ = write!(html, "<p class=\"muted\">Includes {count} with equivalent package contents: Excel comparison omitted. These files count as unchanged, not visually verified.</p>");
    }
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
    fn hash_screened_results_have_their_own_report_category() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hash-screen-report-{}", std::process::id()));
        let file = json!({
            "identity":"screened","name":"screened.xlsx","complete":true,
            "status":"screened-equal",
            "results":[{"profile":"default","status":"selected regions visually unchanged",
                "checks_complete":false,"sheets":[{"index":0,"name":"Sheet1","status":"same"}]}]
        });
        assert!(is_screened(&file));
        assert!(!is_unchanged(&file));
        let html = fs::read_to_string(write(&dir, &json!({"workbooks":[file]}))?)?;
        assert!(html.contains("<strong>1</strong><span>Hash screened</span>"));
        assert!(html.contains("Hash screening complete · 1 worksheet visually checked"));
        assert!(html.contains("Matching preparation hashes are not a complete visual"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn shortened_workbooks_have_visible_coverage_outside_unchanged_list() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("tall-sheet-report-{}", std::process::id()));
        let file = json!({"identity":"tall","name":"Tall.xlsx","complete":true,"status":"unchanged",
        "results":[{"profile":"default","status":"checked ranges visually unchanged; tall-sheet ranges omitted",
                "checked_sheets":2,"unchanged_sheets":1,
            "sheets":[{"index":1,"name":"CSV data","status":"sampled_same","capture_coverage":{
                "total_tiles":200,"captured_tiles":4,"omitted_tiles":196,
                "omitted_ranges":[{"core_rows":[100,999900],"count":196,"reasons":[]}]
            }}]}]});
        assert!(!is_unchanged(&file));
        assert!(!has_differences(&file));
        let html = fs::read_to_string(write(&dir, &json!({"workbooks":[file]}))?)?;
        assert!(html.contains("Tall.xlsx"));
        assert!(html.contains("id=\"partial-captures\""));
        assert!(html.contains("196 not visually checked"));
        assert!(html.contains("1 unchanged worksheets · 0 with differences"));
        assert!(html.contains("<strong>0</strong><span>Unchanged</span>"));
        assert!(!html.contains("id=\"unchanged\""));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn groups_whitespace_last_and_prioritizes_residual_evidence_without_splitting_files(
    ) -> Result<()> {
        let dir = std::env::temp_dir().join(format!("whitespace-report-{}", std::process::id()));
        let white = json!({"identity":"white","name":"white.xlsx","complete":true,"status":"differences",
            "results":[{"profile":"maximum-cleaning","status":"visual differences","sheets":[
                {"index":1,"name":"White","status":"changed","image_differences":[{"range":"A1:B2"}]}]}],
            "whitespace_diagnosis":{"version":1,"status":"complete","profiles":[{"profile":"maximum-cleaning",
                "sheets":[{"index":1,"category":"whitespace_explained","checks_complete":true,"original_evidence_reproduced":true}]}]}});
        let mut mixed = white.clone();
        mixed["identity"] = json!("mixed");
        mixed["name"] = json!("mixed.xlsx");
        mixed["results"][0]["sheets"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "index":2,"name":"Other","status":"changed","image_differences":[{"range":"A1:B2"}]}));
        mixed["whitespace_diagnosis"]["profiles"][0]["control"] = json!({"sheets":[
            {"index":2,"status":"changed","image_differences":[{"range":"A1:B2"}]}]});
        let height = json!({"identity":"height","name":"height.xlsx","complete":true,"status":"differences",
            "results":[{"profile":"maximum-cleaning","sheets":[{"index":1,"name":"Height","status":"changed",
                "row_heights":[{"row":1,"before":12,"after":14}]}]}]});
        let html = fs::read_to_string(write(&dir, &json!({"workbooks":[white,height,mixed]}))?)?;
        let main = html.split("<main").nth(1).unwrap();
        let position = |id| {
            main.find(&format!("id=\"{}\"", anchor("file", id)))
                .unwrap()
        };
        assert!(position("mixed") < position("height") && position("height") < position("white"));
        let mixed_section = &main[position("mixed")..position("height")];
        assert!(mixed_section.find("Sheet 02").unwrap() < mixed_section.find("Sheet 01").unwrap());
        assert!(
            mixed_section.find("remaining differences").unwrap()
                < mixed_section.find("<h3>maximum-cleaning").unwrap()
        );
        assert!(mixed_section.contains("data-change-category=\"other\" open"));
        assert!(main.contains("data-change-category=\"whitespace\" data-search="));
        assert!(html.contains("data-collapse-category=\"height\" checked"));
        assert!(html.contains("remaining visual differences not fully checked"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn legacy_results_get_unknown_whitespace_cause_without_changing_evidence() {
        for status in ["unchanged", "differences", "skipped", "pending"] {
            let mut record = json!({"status":status,"results":[
                {"profile":"maximum-cleaning","status":"visual differences"}
            ]});
            let original = record.clone();
            ensure_whitespace_diagnosis(&mut record);
            assert_eq!(record["whitespace_diagnosis"]["status"], "not_investigated");
            let once = record.clone();
            ensure_whitespace_diagnosis(&mut record);
            assert_eq!(record, once);
            record
                .as_object_mut()
                .unwrap()
                .remove("whitespace_diagnosis");
            assert_eq!(record, original);
        }
        let mut explicit =
            json!({"whitespace_diagnosis":{"status":"not_investigated","note":"retained"}});
        let original = explicit.clone();
        ensure_whitespace_diagnosis(&mut explicit);
        assert_eq!(explicit, original);
    }
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
        assert!(html.contains("Whitespace cause not yet investigated"));
        assert_eq!(
            html.matches("data-whitespace-diagnosis=\"not-investigated\"")
                .count(),
            1
        );
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
    fn package_equivalence_is_distinguished_from_visual_verification() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("package-report-{}", std::process::id()));
        let profiles: Vec<_> = ["default", "maximum-cleaning", "too-aggressive-cleaning"]
            .into_iter()
            .map(|profile| {
                json!({"profile":profile,"status":"package unchanged",
                "checks_complete":false,"sheets":[],"shrink_seconds":1.25,
                "original_bytes":100,"shrunk_bytes":90,
                "image_comparison":"not run; package contents equivalent"})
            })
            .collect();
        let report = json!({"complete":true,
            "corpora":[{"id":"a","name":"Corpus A"},{"id":"b","name":"Corpus B"}],
            "workbooks":[
                {"identity":"package","name":"package.xlsx","corpus_id":"a",
                    "complete":true,"status":"unchanged","capture_mode":"package-audit",
                    "baseline_repaired":true,
                    "package_identity":{"version":1,"status":"equivalent"},"results":profiles},
                {"identity":"visual","name":"visual.xlsx","corpus_id":"b",
                    "complete":true,"status":"unchanged","results":[
                        {"profile":"default","status":"visually unchanged","checks_complete":true,
                            "original_open_seconds":2.0,"shrunk_open_seconds":1.0}]}]});
        let all = fs::read_to_string(write(&dir, &report)?)?;
        assert!(
            all.contains("2 unchanged · 0 with differences · 0 hash screened · 0 skipped/errors")
        );
        assert!(all.contains("Includes 1 with equivalent package contents"));
        assert!(!all.contains("visually unchanged workbooks"));
        let a = fs::read_to_string(dir.join(format!("{}.html", anchor("corpus", "a"))))?;
        assert!(a.contains("Equivalent package contents – Excel comparison omitted"));
        assert!(a.contains("not visually verified"));
        assert!(a.contains("checked against the prepared baseline"));
        assert!(a.contains("The workbook.xml baseline was repaired before shrinking"));
        assert!(a.contains("1.25 s"));
        assert!(a.contains("10.00%"));
        assert_eq!(a.matches("package unchanged").count(), 3);
        assert!(a.contains("<td>— → —</td><td>— → —</td>"));
        assert!(!a.contains("visually unchanged"));
        assert!(!a.contains("0.00 s"));
        assert!(!a.contains("0.0 MiB"));
        let b = fs::read_to_string(dir.join(format!("{}.html", anchor("corpus", "b"))))?;
        assert!(b.contains("visually unchanged"));
        assert!(b.contains("2.00 s → 1.00 s"));
        assert!(!b.contains("Includes 1 with equivalent package contents"));
        assert!(!b.contains("Excel comparison omitted"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn incomplete_or_changed_package_checks_are_not_counted_as_equivalent() {
        let package = json!({"complete":true,"status":"unchanged",
            "capture_mode":"package-audit","package_identity":{"status":"equivalent"}});
        assert!(is_package_unchanged(&package));
        for (key, value) in [
            ("complete", json!(false)),
            ("status", json!("skipped")),
            ("status", json!("differences")),
            ("capture_mode", json!("resident")),
            ("package_identity", json!({"status":"different"})),
        ] {
            let mut other = package.clone();
            other[key] = value;
            assert!(!is_package_unchanged(&other));
        }
    }

    #[test]
    fn machine_groups_and_manifest_links_are_scoped_to_each_corpus_page() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("machine-corpus-report-{}", std::process::id()));
        let run_a = "00000000-0000-4000-8000-000000000001";
        let run_b = "00000000-0000-4000-8000-000000000002";
        let report = json!({"complete":true,"corpora":[{"id":"a","name":"Corpus A"},{"id":"b","name":"Corpus B"}],
            "measurement_provenance":{"runs":[
                {"run_id":run_a,"machine_id":"tower","machine_manifest":"metadata/machines/tower-a.json"},
                {"run_id":run_b,"machine_id":"laptop","machine_manifest":"metadata/machines/laptop-b.json"}],
                "machines":[{"path":"metadata/machines/tower-a.json","metadata":{"machine_id":"tower","label":"Tower A"}},
                    {"path":"metadata/machines/laptop-b.json","metadata":{"machine_id":"laptop","label":"Laptop B"}}]},
            "workbooks":[
                {"identity":"a:0:a.xlsx","name":"alpha.xlsx","corpus_id":"a","complete":true,"status":"unchanged",
                    "provenance":{"comparison":{"machine_id":"tower","run_id":run_a}},"results":[]},
                {"identity":"b:0:b.xlsx","name":"beta.xlsx","corpus_id":"b","complete":true,"status":"skipped",
                    "provenance":{"comparison":{"machine_id":"laptop","run_id":run_b}},"results":[]}]});
        let all = fs::read_to_string(write(&dir, &report)?)?;
        assert!(all.contains("Tower A"));
        assert!(all.contains("Laptop B"));
        let a = fs::read_to_string(dir.join(format!("{}.html", anchor("corpus", "a"))))?;
        assert!(a.contains("Tower A"));
        assert!(!a.contains("Laptop B"));
        assert!(!a.contains("beta.xlsx"));
        assert!(a.contains(&format!("href=\"metadata/runs/{run_a}.json\"")));
        assert!(a.contains("href=\"metadata/machines/tower-a.json\""));
        assert!(!a.contains(run_b));
        let anchor = file_id(&report["workbooks"][0]);
        assert!(a.contains(&format!("href=\"#{anchor}\"")));
        assert!(a.contains(&format!("id=\"{anchor}\"")));
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
        assert!(worksheet.contains("<h3>too-aggressive-cleaning</h3>"));
        assert!(!worksheet.contains("<h3>maximum-cleaning</h3>"));
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
        assert!(
            html.contains("1 unchanged · 0 with differences · 0 hash screened · 1 skipped/errors")
        );
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
