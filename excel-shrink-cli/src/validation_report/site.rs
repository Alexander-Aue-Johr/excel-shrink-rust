//! A read-only static view of immutable report packages, without a running API.

use super::{evidence, read_json_file, FORMAT_VERSION};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const TEMPLATE: &str = include_str!("../../office-capture/control-room/report.html");
const OBSERVATORY_CSS: &str = include_str!("../../office-capture/control-room/style.css");
const REPORT_CSS: &str = include_str!("../../office-capture/control-room/report.css");
const SCRIPTS: [&str; 4] = [
    include_str!("../../office-capture/control-room/atelier-model.js"),
    include_str!("../../office-capture/control-room/pixel-summary.js"),
    include_str!("../../office-capture/control-room/atelier-pixels.js"),
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
        "error": run["error"],
        "interrupted_index": run["interrupted_index"],
        "pairs": rows
    });
    let path = root.join("index.html");
    write_derived(
        &root,
        &path,
        render_page("DeltaSieve validation report", "", &data)?.as_bytes(),
    )?;
    Ok(path)
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
        "kind": "comparison", "title": label, "back": format!("{prefix}index.html"),
        "report": report
    });
    let page = report_path.with_file_name("index.html");
    write_derived(root, &page, render_page(label, &prefix, &data)?.as_bytes())?;
    Ok(page)
}

/// Export only run metadata and verified comparison packages into a new folder.
/// Original reports stay read-only. Processing outputs, scratch workbooks,
/// telemetry histories and executable programs are deliberately not exported.
pub(crate) fn export_site(source_root: &Path, destination: &Path) -> Result<PathBuf> {
    let source = source_root
        .canonicalize()
        .context("Resolve source report")?;
    let mut run = crate::validation::read_run(&source)?;
    // The exported index is a complete snapshot of the recovered journal. Its
    // validation status is unchanged, including interrupted or failed runs.
    run["index_complete"] = json!(true);
    let rows = overview_rows(&source, &run)?;
    let destination = evidence::create_destination(&source, destination)?;
    let mut identities = BTreeSet::new();
    for row in rows {
        if let Some(relative) = row["identity"].as_str() {
            if identities.insert(relative.to_owned()) {
                evidence::copy_identity(
                    &source,
                    &destination,
                    relative,
                    row["identity_sha256"].as_str(),
                )?;
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
    }
    evidence::write_new(
        &destination.join("run.json"),
        &serde_json::to_vec_pretty(&run)?,
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
            "id": id, "name": pair["name"].as_str().unwrap_or(id),
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

/// These four small, local modules have single-line named imports. Removing
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

fn ensure_owned_directory(root: &Path, directory: &Path) -> Result<()> {
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

fn write_derived(root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
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
    fn workbook_text_cannot_close_the_data_script_or_change_markup() -> Result<()> {
        let attack = "</script><script>alert('workbook')</script>&";
        let html = render_page(attack, "../../", &json!({"name": attack}))?;
        assert!(!html.contains(attack));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("&lt;/script&gt;"));
        assert!(!html.contains("{{DATA}}"));
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
        assert!(!script.contains("fetch("));
        Ok(())
    }
}
