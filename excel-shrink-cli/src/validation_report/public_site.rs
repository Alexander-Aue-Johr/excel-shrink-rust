//! A derived Atlas publication, separate from reusable native evidence packages.

use super::{evidence, publication, read_json_file, site};
use anyhow::{ensure, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};

/// Export the current static Atlas without copying run configurations or replay
/// archives. Scientific finding payloads and encoded images remain byte-exact.
/// Source hashes identify the private, unmodified evidence used for this view.
pub(crate) fn export(source: &Path, destination: &Path) -> Result<()> {
    let source = source.canonicalize()?;
    ensure!(
        !source.join(".incomplete").exists(),
        "Source export is incomplete"
    );
    let mut catalog = read_gzip(&source, "atlas-data/catalog.json.gz")?;
    ensure!(catalog["schema"] == 1, "Unsupported Atlas schema");
    let destination = evidence::create_destination(&source, destination)?;
    evidence::write_new(
        &destination.join(".incomplete"),
        b"Public Atlas export in progress",
    )?;
    let index = fs::read_to_string(evidence::confined_file(&source, "index.html")?)?;
    let index = index.replace(
        "href=\"./report.html\">DETAILED REPORT</a>",
        "href=\"./public-provenance.json\">PROVENANCE</a>",
    );
    write(&destination, "index.html", index.as_bytes())?;
    // Existing per-comparison pages return here using their ordinary back link.
    write(
        &destination,
        "report.html",
        br#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="0;url=./index.html">
  <title>Public comparison Atlas</title>
</head>
<body><a href="./index.html">Return to the public comparison Atlas</a></body>
</html>
"#,
    )?;
    copy(&source, &destination, ".nojekyll")?;
    for directory in ["atlas-assets", "report-assets"] {
        for entry in walkdir::WalkDir::new(source.join(directory)) {
            let entry = entry?;
            ensure!(
                !entry.file_type().is_symlink(),
                "Publication assets cannot be symlinks"
            );
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&source)?
                    .to_string_lossy()
                    .replace('\\', "/");
                copy(&source, &destination, &relative)?;
            }
        }
    }
    let mut provenance = Vec::new();
    let mut catalog_omissions = Vec::new();
    let mut copied_details = BTreeSet::new();
    for book in catalog["books"]
        .as_array_mut()
        .context("Atlas has no workbook list")?
    {
        let book_name = book["name"].as_str().unwrap_or("Comparison").to_owned();
        for observation in book["observations"]
            .as_array_mut()
            .context("Workbook has no observation list")?
        {
            let observation_id = observation["id"]
                .as_str()
                .context("Missing observation ID")?
                .to_owned();
            let mut omissions = Vec::new();
            project_metadata(observation, "", &mut omissions);
            catalog_omissions
                .push(json!({"observation":observation_id,"metadata_omissions":omissions}));
            for sheet in observation["sheets"]
                .as_object()
                .into_iter()
                .flat_map(|s| s.values())
            {
                if let Some(relative) = sheet["thumbnail"].as_str() {
                    if !destination.join(relative).exists() {
                        copy(&source, &destination, relative)?;
                    }
                }
            }
            let Some(relative) = observation["detail"].as_str() else {
                continue;
            };
            if !copied_details.insert(relative.to_owned()) {
                continue;
            }
            let mut detail = read_gzip(&source, relative)?;
            let image_base = detail["image_base"]
                .as_str()
                .context("Missing image base")?
                .to_owned();
            let report_relative = format!("{image_base}report.json");
            let report_path = evidence::confined_file(&source, &report_relative)?;
            let package_root = report_path.parent().context("Missing comparison package")?;
            let manifest = evidence::read_verified_package_manifest(package_root)?;
            ensure!(
                read_json_file(&report_path)? == detail["report"],
                "Atlas detail differs from its source report"
            );
            for (image, identity) in &manifest.images {
                let relative = format!("{image_base}{image}");
                let original = evidence::confined_file(&source, &relative)?;
                let target = destination.join(evidence::relative_path(&relative)?);
                if !target.exists() {
                    fs::create_dir_all(target.parent().context("Missing image parent")?)?;
                    fs::copy(&original, &target)?;
                }
                evidence::verify_file(&target, identity)?;
            }
            let mut omitted = Vec::new();
            project_metadata(&mut detail, "", &mut omitted);
            let source_sha = file_hash(&report_path)?;
            detail["public_presentation"] = json!({"schema":1,"source_report_sha256":source_sha,
                "metadata_omissions":omitted,"meaning":"Derived presentation. Native evidence remains unchanged."});
            detail["report"]["public_presentation"] = detail["public_presentation"].clone();
            write_gzip(&destination, relative, &detail)?;
            write(
                &destination,
                &report_relative,
                // Keep generated reports compact so formatting cannot push a
                // previously readable package over the report-size bound.
                &serde_json::to_vec(&detail["report"])?,
            )?;
            site::write_public_pair_page(&destination, &report_relative, &book_name)?;
            provenance.push(json!({"detail":relative,"source_report_sha256":source_sha,"metadata_omissions":omitted}));
        }
    }
    catalog["public_presentation"] = json!({"schema":1,"source_catalog_sha256":file_hash(
        &evidence::confined_file(&source, "atlas-data/catalog.json.gz")?)?,
        "meaning":"Derived static presentation. This directory is not a reusable native validation run."});
    let mut bytes = 0u64;
    for entry in walkdir::WalkDir::new(&destination) {
        let entry = entry?;
        if entry.file_type().is_file() {
            bytes += entry.metadata()?.len();
        }
    }
    catalog["storage"] = json!({"bytes_before_index":bytes,
        "github_pages_budget_bytes":1_000_000_000u64,
        "within_budget":bytes + 1024 * 1024 < 1_000_000_000u64});
    write_gzip(&destination, "atlas-data/catalog.json.gz", &catalog)?;
    write(
        &destination,
        "public-provenance.json",
        &serde_json::to_vec_pretty(&json!({
        "schema":1,"kind":"public_atlas_presentation","observations":provenance,"catalog_omissions":catalog_omissions,
        "limitations":["Review corpus distribution rights, workbook names, formulas and image contents before publication.",
            "Local diagnostic text is omitted. Scientific finding payloads and encoded images are preserved.",
            "Native run metadata and replay archives remain in the original evidence export."]}))?,
    )?;
    let review = publication::audit_tree(&destination)?;
    write(
        &destination,
        "publication-review.json",
        &serde_json::to_vec_pretty(&review)?,
    )?;
    fs::remove_file(destination.join(".incomplete"))?;
    Ok(())
}

fn read_gzip(root: &Path, relative: &str) -> Result<Value> {
    let path = evidence::confined_file(root, relative)?;
    ensure!(
        fs::metadata(&path)?.len() <= super::MAX_REPORT_BYTES,
        "Compressed Atlas data exceeds budget"
    );
    let mut bytes = Vec::new();
    GzDecoder::new(fs::File::open(path)?)
        .take(super::MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= super::MAX_REPORT_BYTES,
        "Expanded Atlas data exceeds budget"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_gzip(root: &Path, relative: &str, value: &Value) -> Result<()> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&serde_json::to_vec(value)?)?;
    write(root, relative, &encoder.finish()?)
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(evidence::relative_path(relative)?);
    fs::create_dir_all(path.parent().context("Missing destination parent")?)?;
    evidence::write_new(&path, bytes)
}

fn copy(source: &Path, destination: &Path, relative: &str) -> Result<()> {
    let original = evidence::confined_file(source, relative)?;
    let target = destination.join(evidence::relative_path(relative)?);
    fs::create_dir_all(target.parent().context("Missing asset parent")?)?;
    fs::copy(&original, &target)?;
    ensure!(
        file_hash(&original)? == file_hash(&target)?,
        "Publication asset copy differs"
    );
    Ok(())
}

fn file_hash(path: &Path) -> Result<String> {
    deltasieve_engine::reuse::file_sha256(path)
}

/// Omit diagnostic metadata only. Formulas, rendered geometry, pixel findings
/// and dependency decisions are scientific data and require separate review.
fn project_metadata(value: &mut Value, pointer: &str, omissions: &mut Vec<Value>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                if matches!(
                    key.as_str(),
                    "comparisons"
                        | "formula_diagnostics"
                        | "chart_dependencies"
                        | "conditional_formatting"
                ) {
                    continue;
                }
                let path = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                if !child.is_null()
                    && (publication::sensitive_field(key)
                        || matches!(
                            key.as_str(),
                            "excel_pids"
                                | "before_file"
                                | "after_file"
                                | "changes_file"
                                | "source_report"
                        )
                        || matches!(key.as_str(), "error" | "errors"))
                {
                    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(child).unwrap()));
                    omissions.push(json!({"pointer":path,"source_value_sha256":hash}));
                    // Keep environment distinctions in cross-tool context matching
                    // without publishing the original machine identifier.
                    *child = if key == "environment_key" {
                        json!(format!("sha256:{hash}"))
                    } else {
                        Value::Null
                    };
                } else {
                    project_metadata(child, &path, omissions);
                }
            }
        }
        Value::Array(children) => {
            for (index, child) in children.iter_mut().enumerate() {
                project_metadata(child, &format!("{pointer}/{index}"), omissions);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_scientific_payload_and_records_metadata_hashes() {
        let mut value = json!({"report":{"comparisons":[{"formula":"C:/source/book.xlsx", "pixels_differ":true}],
            "transitions":[{"before_file":"C:/private/book.xlsx","change_count":12}],
            "excel_pids":{"O":123},"status":"failed","portable_report":{"failure":{"error":"private diagnostic","kind":"name_conflict"}}},
            "validation_signature":{"environment_key":"private-host","capture_policy":"v16"}});
        let original = value.clone();
        let mut omissions = Vec::new();
        project_metadata(&mut value, "", &mut omissions);
        assert_eq!(
            value["report"]["comparisons"],
            original["report"]["comparisons"]
        );
        assert_eq!(value["report"]["transitions"][0]["change_count"], 12);
        assert_eq!(value["report"]["status"], "failed");
        assert_eq!(
            value["report"]["portable_report"]["failure"]["kind"],
            "name_conflict"
        );
        assert!(value["report"]["excel_pids"].is_null());
        assert!(value["validation_signature"]["environment_key"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert_eq!(omissions.len(), 4);
    }
}
