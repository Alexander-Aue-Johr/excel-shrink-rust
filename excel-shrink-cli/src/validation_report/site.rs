//! A read-only static view of immutable report packages, without a running API.

use super::{evidence, read_json_file, FORMAT_VERSION};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const TEMPLATE: &str = include_str!("../../office-capture/control-room/report.html");
const OBSERVATORY_CSS: &str = include_str!("../../office-capture/control-room/style.css");
const REPORT_CSS: &str = include_str!("../../office-capture/control-room/report.css");
const SCRIPTS: [&str; 7] = [
    include_str!("../../office-capture/control-room/atelier-model.js"),
    include_str!("../../office-capture/control-room/pixel-summary.js"),
    include_str!("../../office-capture/control-room/atelier-pixels.js"),
    include_str!("../../office-capture/control-room/formula-inspector.js"),
    include_str!("../../office-capture/control-room/replay-engine.js"),
    include_str!("../../office-capture/control-room/report-replay.js"),
    include_str!("../../office-capture/control-room/report.js"),
];

/// Publish an overview and the existing comparisons as a static website.
/// `run.pairs` entries contain `id`, `name`, `status`, and an optional
/// report-relative `report` path. Missing reports remain visible as incomplete.
///
/// This writes derived HTML/assets only. It never updates scientific reports,
/// run.json, workbook inputs, or telemetry. Call once at completion/export;
/// use [`write_pair_page`] to expose one newly completed comparison during a run.
pub(crate) fn write_site(report_root: &Path, run: &Value) -> Result<PathBuf> {
    let root = report_root.canonicalize().context("Resolve report root")?;
    ensure_viewer_assets(&root)?;
    let rows = overview_rows(&root, run)?;
    for row in &rows {
        if let Some(relative) = row["report"].as_str() {
            write_pair_page_contents(
                &root,
                relative,
                row["name"].as_str().unwrap_or("Comparison"),
            )?;
        }
    }
    let data = json!({
        "kind": "run",
        "title": "DeltaSieve validation report",
        "run_id": run["run_id"],
        "status": run["status"],
        "created_at": run.get("created_at").filter(|value| !value.is_null()).unwrap_or(&run["started_at"]),
        "finished_at": run["finished_at"],
        "summary": run["summary"],
        "selection": run["selection"],
        "observation_selection": run["observation_selection"],
        "preparation": run["preparation"],
        "replay": run["replay"],
        "merge": run["merge"],
        "error": run["error"],
        "interrupted_index": run["interrupted_index"],
        "publication_review": if root.join("publication-review.json").is_file() {
            Some(read_json_file(&evidence::confined_file(&root, "publication-review.json")?)?)
        } else { None },
        "pairs": rows
    });
    let path = root.join("report.html");
    write_derived(
        &root,
        &path,
        render_page("DeltaSieve validation report", "", &data)?.as_bytes(),
    )?;
    super::atlas::write(&root, run)
}

/// Create only one pair's viewer page. The shared region model, pixel inspector,
/// and styles are also used by the live Observatory. No images are decoded here.
pub(crate) fn write_pair_page(
    report_root: &Path,
    report_relative: &str,
    label: &str,
) -> Result<PathBuf> {
    let root = report_root.canonicalize()?;
    ensure_viewer_assets(&root)?;
    write_pair_page_contents(&root, report_relative, label)
}

fn write_pair_page_contents(root: &Path, report_relative: &str, label: &str) -> Result<PathBuf> {
    let report_path = evidence::confined_file(root, report_relative)?;
    ensure!(
        report_path
            .file_name()
            .is_some_and(|name| name == "report.json"),
        "A comparison page requires report.json"
    );
    let parent = evidence::relative_path(report_relative)?
        .parent()
        .context("Missing comparison directory")?;
    ensure!(
        !parent.as_os_str().is_empty(),
        "A comparison page must be below the run root"
    );
    let report = read_json_file(&report_path)?;
    ensure!(
        report["portable_report"]["version"] == FORMAT_VERSION,
        "Comparison has not been packaged for portable viewing"
    );
    let prefix = "../".repeat(parent.components().count());
    let data = json!({
        "kind": "comparison", "title": label, "back": format!("{prefix}report.html"),
        "report": report
    });
    let page = report_path.with_file_name("index.html");
    write_derived(root, &page, render_page(label, &prefix, &data)?.as_bytes())?;
    Ok(page)
}

/// Export run metadata and verified packages with a publication-review inventory.
/// Original reports stay read-only. Processing outputs, scratch workbooks,
/// Raw telemetry histories and executable programs are deliberately not exported.
/// The compact replay channel is verified separately before publication.
pub(crate) fn export_site(source_root: &Path, destination: &Path) -> Result<PathBuf> {
    export_selected_site(source_root, destination, &[])
}

/// Select observations without changing their source reports or scientific verdicts.
pub(crate) fn export_selected_site(
    source_root: &Path,
    destination: &Path,
    pair_ids: &[String],
) -> Result<PathBuf> {
    let source = source_root
        .canonicalize()
        .context("Resolve source report")?;
    let original_metadata = fs::read(source.join("run.json"))?;
    let mut run = crate::validation::read_run(&source)?;
    ensure!(
        original_metadata == fs::read(source.join("run.json"))?,
        "Source run metadata changed during export inspection"
    );
    if run["merge"].is_object() {
        ensure!(pair_ids.is_empty(), "Select pairs before merging reports");
        return super::merge::export_collection(&source, &run, destination);
    }
    if !pair_ids.is_empty() {
        ensure!(
            run["observation_selection"].is_null(),
            "Select from the original run, not a previous selection"
        );
        let selected: BTreeSet<_> = pair_ids.iter().map(String::as_str).collect();
        ensure!(selected.len() == pair_ids.len(), "Duplicate selected pair ID");
        let pairs = run["pairs"]
            .as_array()
            .context("Run report has no pair list")?;
        let retained: Vec<_> = pairs
            .iter()
            .filter(|pair| pair["id"].as_str().is_some_and(|id| selected.contains(id)))
            .cloned()
            .collect();
        let source_pair_count = pairs.len();
        let failed = retained
            .iter()
            .filter(|pair| !matches!(pair["status"].as_str(), Some("completed_with_limitations" | "reused")))
            .count();
        ensure!(
            retained.len() == selected.len(),
            "Selected pair IDs are missing or ambiguous in the source report"
        );
        ensure!(
            retained.iter().all(|pair| matches!(
                pair["status"].as_str(),
                Some("completed_with_limitations" | "reused" | "failed")
            )),
            "Finish each selected comparison before exporting it"
        );
        run["observation_selection"] = json!({
            "source_run_sha256": format!("{:x}", sha2::Sha256::digest(&original_metadata)),
            "source_metadata": "selection-source-run.json",
            "pair_ids": pair_ids,
            "source_pair_count": source_pair_count,
            "source_summary": run["summary"],
            "source_status": run["status"],
            "meaning": "Selected recorded observations. Original run status and timing describe the full source run."
        });
        run["summary"] = json!({
            "total": retained.len(),
            "processed_pairs": retained.len() - failed,
            "failed_pairs": failed
        });
        run["pairs"] = json!(retained);
        run["status"] = json!(if failed == 0 {
            "completed_with_limitations"
        } else {
            "completed_with_errors"
        });
    }
    // The exported index is a complete snapshot of the recovered journal. Its
    // validation status is unchanged, including interrupted or failed runs.
    run["index_complete"] = json!(true);
    let config = run["config"].clone();
    for pair in run["pairs"].as_array_mut().context("Run report has no pair list")? {
        if pair["workbook_inventory"]["complete"] == true {
            continue;
        }
        if let Some(inventory) = pair["name"]
            .as_str()
            .and_then(|name| super::inventory::recover(&config, name))
        {
            pair["workbook_inventory"] = inventory;
        }
    }
    let rows = overview_rows(&source, &run)?;
    let destination = evidence::create_destination(&source, destination)?;
    super::atlas::copy_labels(&source, &destination)?;
    let mut identities = BTreeSet::new();
    let mut audit_files = vec![String::from("run.json")];
    if run["observation_selection"].is_object() {
        let metadata = if pair_ids.is_empty() {
            fs::read(evidence::confined_file(&source, "selection-source-run.json")?)?
        } else {
            original_metadata
        };
        ensure!(
            run["observation_selection"]["source_run_sha256"] == format!("{:x}", sha2::Sha256::digest(&metadata)),
            "Selection source metadata checksum changed"
        );
        evidence::write_new(&destination.join("selection-source-run.json"), &metadata)?;
        audit_files.push("selection-source-run.json".to_owned());
    }
    for row in rows {
        if let Some(relative) = row["identity"].as_str() {
            if identities.insert(relative.to_owned()) {
                evidence::copy_identity(
                    &source,
                    &destination,
                    relative,
                    row["identity_sha256"].as_str(),
                )?;
                audit_files.push(relative.to_owned());
            }
        }
        let Some(relative) = row["report"].as_str() else {
            continue;
        };
        let report = evidence::confined_file(&source, relative)?;
        let package = report
            .parent()
            .context("Missing source comparison directory")?;
        let report_target = destination.join(evidence::relative_path(relative)?);
        let target = report_target
            .parent()
            .context("Missing exported comparison directory")?;
        evidence::copy_verified_package(package, &target)?;
        audit_files.push(relative.to_owned());
    }
    let (replay, replay_files) = if run["observation_selection"].is_object() {
        (json!({"available": false, "reason": "Source replay covers a wider run than the selected observations."}), Vec::new())
    } else {
        super::replay::copy_channel(&source, &destination)?
    };
    run["replay"] = replay;
    audit_files.extend(replay_files);
    evidence::write_new(
        &destination.join("run.json"),
        &serde_json::to_vec_pretty(&run)?,
    )?;
    let review = super::publication::audit(&destination, &audit_files)?;
    evidence::write_new(
        &destination.join("publication-review.json"),
        &serde_json::to_vec_pretty(&review)?,
    )?;
    write_site(&destination, &run)
}

fn overview_rows(root: &Path, run: &Value) -> Result<Vec<Value>> {
    let pairs = run["pairs"]
        .as_array()
        .context("Run report has no pair list")?;
    let mut identities = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut rows = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let id = pair["id"].as_str().context("Comparison entry has no id")?;
        ensure!(
            identities.insert(id),
            "Duplicate comparison id in report: {id}"
        );
        let report = pair["report"].as_str();
        let page = if let Some(relative) = report {
            let relative_path = evidence::relative_path(relative)?;
            ensure!(
                relative_path
                    .file_name()
                    .is_some_and(|name| name == "report.json")
                    && relative_path
                        .parent()
                        .is_some_and(|path| !path.as_os_str().is_empty()),
                "Invalid pair report path: {relative}"
            );
            evidence::confined_file(root, relative)?;
            ensure!(
                paths.insert(relative),
                "Two pairs refer to the same report path: {relative}"
            );
            Some(
                relative_path
                    .with_file_name("index.html")
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        } else {
            None
        };
        rows.push(json!({
            "id": id, "name": pair["display_name"].as_str().or_else(|| pair["name"].as_str()).unwrap_or(id),
            "preparation":pair["preparation"], "merge_source":pair["merge_source"],
            "status": pair["status"], "report": report, "page": page,
            "different_regions": pair["different_regions"], "reuse": pair["reuse"],
            "error": pair["error"], "errors": pair["errors"], "identity": pair["identity"],
            "identity_sha256": pair["identity_sha256"]
        }));
    }
    Ok(rows)
}

fn ensure_viewer_assets(root: &Path) -> Result<()> {
    let directory = root.join("report-assets");
    if !directory.try_exists()? {
        fs::create_dir(&directory)?;
    }
    ensure_owned_directory(root, &directory)?;
    write_derived(
        root,
        &directory.join("observatory.css"),
        OBSERVATORY_CSS.as_bytes(),
    )?;
    write_derived(root, &directory.join("report.css"), REPORT_CSS.as_bytes())?;
    write_derived(
        root,
        &directory.join("report-view.js"),
        browser_bundle()?.as_bytes(),
    )?;
    Ok(())
}

/// These small, local modules have single-line named imports. Removing
/// their module declarations lets the same sources run as one classic script,
/// including from file:// where cross-file ES module loading is restricted.
fn browser_bundle() -> Result<String> {
    let mut result = String::from("(function () {\n'use strict';\n");
    for script in SCRIPTS {
        for line in script.lines() {
            if line.starts_with("import ") {
                ensure!(
                    line.ends_with(';'),
                    "Portable viewer requires a single-line local import"
                );
                continue;
            }
            result.push_str(line.strip_prefix("export ").unwrap_or(line));
            result.push('\n');
        }
    }
    result.push_str("}());\n");
    Ok(result)
}

fn render_page(title: &str, prefix: &str, data: &Value) -> Result<String> {
    let data = serde_json::to_string(data)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let title = html_text(title);
    let mut rendered = String::new();
    let mut remaining = TEMPLATE;
    while let Some((before, token)) = remaining.split_once("{{") {
        rendered.push_str(before);
        let (name, after) = token.split_once("}}").context("Invalid report template")?;
        rendered.push_str(match name {
            "TITLE" => &title,
            "ASSET_PREFIX" => prefix,
            "DATA" => &data,
            _ => anyhow::bail!("Unknown report template field: {name}"),
        });
        remaining = after;
    }
    rendered.push_str(remaining);
    Ok(rendered)
}

fn html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(super) fn ensure_owned_directory(root: &Path, directory: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Viewer output must be an ordinary directory"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Viewer output cannot be a reparse point"
        );
    }
    ensure!(
        directory.canonicalize()?.starts_with(root),
        "Viewer output escapes report root"
    );
    Ok(())
}

pub(super) fn write_derived(root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_owned_directory(root, path.parent().context("Viewer output has no parent")?)?;
    if path.try_exists()? {
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let file = evidence::confined_file(root, &relative)?;
        if file.metadata()?.len() == bytes.len() as u64 && fs::read(&file)?.as_slice() == bytes {
            return Ok(());
        }
    }
    crate::regression::write_atomic_artifact(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_display_names_do_not_replace_technical_pair_identity() -> Result<()> {
        let root = std::env::temp_dir().join(format!("imported-site-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let run = json!({"pairs":[{"id":"pair-1","name":"technical.xlsx","display_name":"Original workbook.xls",
            "status":"pending","preparation":{"source_id":"corpus:7:book.xls","producer_measurements":{"open_ms":42}}}]});
        let rows = overview_rows(&root, &run)?;
        assert_eq!(rows[0]["id"], "pair-1");
        assert_eq!(rows[0]["name"], "Original workbook.xls");
        assert_eq!(rows[0]["preparation"]["source_id"], "corpus:7:book.xls");
        assert_eq!(rows[0]["preparation"]["producer_measurements"]["open_ms"], 42);
        Ok(())
    }

    #[test]
    fn workbook_text_cannot_close_the_data_script_or_change_markup() -> Result<()> {
        let attack = "</script><script>alert('workbook')</script>&";
        let html = render_page(attack, "../../", &json!({"name": attack}))?;
        assert!(!html.contains(attack));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("&lt;/script&gt;"));
        assert!(!html.contains("{{DATA}}"));
        assert!(html.contains("connect-src 'self'"));
        assert!(html.contains("default-src 'none'"));
        let literal = render_page("{{DATA}}", "", &json!({"name":"{{TITLE}}"}))?;
        assert!(literal.contains("<title>{{DATA}}</title>"));
        assert!(literal.contains(r#""name":"{{TITLE}}""#));
        Ok(())
    }

    #[test]
    fn portable_bundle_has_no_imports_and_keeps_the_shared_inspector() -> Result<()> {
        let script = browser_bundle()?;
        assert!(!script
            .lines()
            .any(|line| line.starts_with("import ") || line.starts_with("export ")));
        assert!(script.contains("function createPixelInspector("));
        assert!(script.contains("function atelierRegions("));
        assert!(script.contains("function createRetainedFormulaInspector("));
        // Replay is the sole fetch entry point in the otherwise self-contained viewer.
        assert_eq!(script.matches("fetch(").count(), 1);
        assert!(script.contains("class ReplayReader"));
        assert!(script.contains("redirect: 'error'"));
        Ok(())
    }
}
