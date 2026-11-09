//! Content-addressed, immutable validation artifacts and explicit package audits.
//! Hash ZIP *members*, not ZIP ordering, compression or timestamps. Raw archive
//! hashes are only used to avoid inflating the same immutable archive repeatedly.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::{BTreeMap, BTreeSet}, fs::{self, File}, io::{Read, Write}, path::{Path, PathBuf}};
use zip::{ZipArchive, ZipWriter};

mod scan;

const SCHEMA: u32 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PartDigest {
    pub bytes: u64,
    pub sha256: String,
    pub visual_context: bool,
    pub visual_runtime: bool,
    pub recalc_cacheable: bool,
    pub has_formula: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PackageManifest {
    pub archive_sha256: String,
    pub package_sha256: String,
    pub parts: BTreeMap<String, PartDigest>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub path: PathBuf,
    pub manifest: PackageManifest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedBaseline {
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub repair_key: String,
    pub snapshot: Snapshot,
    pub workbook_xml_changed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PackageChange {
    pub part: String,
    pub allowed: bool,
    pub before: Option<PartDigest>,
    pub after: Option<PartDigest>,
    pub diff_directory: Option<PathBuf>,
}

pub(crate) fn root() -> Result<PathBuf> {
    Ok(match std::env::var_os("EXCEL_SHRINK_REGRESSION_CACHE_DIR") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()?.join(".tmp").join("regression-cache"),
    })
}
pub(crate) use corpus_runner::artifacts::{sha256, hex, hash_file, unique_path, atomic_write, atomic_replace};
#[derive(Serialize, Deserialize)]
struct Record { schema: u32, data: serde_json::Value, sha256: String }
pub(crate) fn read_record<T: DeserializeOwned>(path: &Path) -> Option<T> {
    if !path.is_file() { return None; }
    let read = (|| -> Result<T> {
        let rec: Record = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(rec.schema == SCHEMA, "old cache schema");
        ensure!(sha256(&serde_json::to_vec(&rec.data)?) == rec.sha256, "cache checksum mismatch");
        Ok(serde_json::from_value(rec.data)?)
    })();
    match read {
        Ok(value) => Some(value),
        Err(error) => { tracing::warn!("[validation-cache] ignoring {}: {error:#}", path.display()); None }
    }
}
pub(crate) fn write_record<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_value(value)?;
    let digest = sha256(&serde_json::to_vec(&data)?);
    atomic_write(path, &serde_json::to_vec_pretty(&Record { schema: SCHEMA, data, sha256: digest })?)
}

pub(crate) fn is_sheet(name: &str) -> bool {
    name.starts_with("xl/worksheets/") && name.ends_with(".xml") && !name[14..].contains('/')
}
fn package_hash(parts: &BTreeMap<String, PartDigest>) -> String {
    let mut hash = Sha256::new(); hash.update(b"excel-shrink-package-v9\0");
    for (name, part) in parts {
        hash.update((name.len() as u64).to_le_bytes()); hash.update(name.as_bytes());
        hash.update(part.bytes.to_le_bytes()); hash.update(part.sha256.as_bytes());
    }
    hex(&hash.finalize())
}
pub(crate) fn manifest_algorithm_key() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("artifact_cache.rs"));
    hash.update(include_bytes!("artifact_cache/scan.rs"));
    hash.update(include_bytes!("streaming_xml.rs"));
    hash.update(include_bytes!("xml_views.rs"));
    hex(&hash.finalize())
}
pub(crate) fn manifest(path: &Path) -> Result<PackageManifest> { manifest_in(path, &root()?) }
pub(crate) fn manifest_in(path: &Path, cache: &Path) -> Result<PackageManifest> {
    let raw_hash = hash_file(path)?;
    let algorithm = manifest_algorithm_key();
    let record = cache.join("packages").join(&algorithm[..16]).join(format!("{raw_hash}.json"));
    if let Some(manifest) = read_record::<PackageManifest>(&record) {
        if manifest.archive_sha256 == raw_hash && package_hash(&manifest.parts) == manifest.package_sha256 {
            return Ok(manifest);
        }
    }
    let _phase = super::visual_diagnostics::phase("validation.package_manifest", path.display().to_string());
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let mut parts = BTreeMap::new();
    for index in 0..zip.len() {
        let mut part = zip.by_index(index)?;
        if part.is_dir() { continue; }
        let name = part.name().to_owned();
        ensure!(!parts.contains_key(&name), "duplicate ZIP member {name}");
        let expected_bytes = part.size();
        let digest = scan::digest_part(&name, &mut part, expected_bytes)
            .with_context(|| format!("stream ZIP member {name}"))?;
        parts.insert(name, digest);
    }
    drop(zip);
    ensure!(hash_file(path)? == raw_hash, "workbook changed while indexing {}", path.display());
    let result = PackageManifest { archive_sha256: raw_hash, package_sha256: package_hash(&parts), parts };
    write_record(&record, &result)?;
    Ok(result)
}
pub(crate) fn snapshot(path: &Path) -> Result<Snapshot> { snapshot_in(path, &root()?) }
fn snapshot_in(path: &Path, cache: &Path) -> Result<Snapshot> {
    let manifest = manifest_in(path, cache)?;
    let target = cache.join("objects").join(format!("{}.xlsx", manifest.archive_sha256));
    if !target.is_file() || hash_file(&target)? != manifest.archive_sha256 {
        let pending = unique_path(target.parent().unwrap(), "snapshot")?;
        fs::copy(path, &pending)?;
        ensure!(hash_file(&pending)? == manifest.archive_sha256, "source changed while preserving snapshot");
        atomic_replace(&pending, &target)?;
    }
    Ok(Snapshot { path: target, manifest })
}

pub(crate) fn prepare_baseline(source: &Path) -> Result<PreparedBaseline> {
    let cache = root()?;
    let source_path = fs::canonicalize(source)?;
    let source_sha256 = hash_file(source)?;
    let logical = sha256(source_path.to_string_lossy().as_bytes());
    let mut recipe = Sha256::new();
    recipe.update(include_bytes!("../../../excel-shrink-core/src/workbook.rs"));
    recipe.update(manifest_algorithm_key().as_bytes());
    let repair_key = hex(&recipe.finalize());
    let revision = sha256(format!("{source_sha256}/{repair_key}").as_bytes());
    let directory = cache.join("baselines").join(&logical[..16]).join(&revision[..32]);
    let record = directory.join("baseline.json");
    if let Some(saved) = read_record::<PreparedBaseline>(&record) {
        if saved.source_path == source_path && saved.source_sha256 == source_sha256 && saved.repair_key == repair_key
            && saved.snapshot.path.is_file() && hash_file(&saved.snapshot.path)? == saved.snapshot.manifest.archive_sha256 {
            tracing::info!("[baseline] cache hit {}", saved.snapshot.path.display());
            return Ok(saved);
        }
    }
    let _phase = super::visual_diagnostics::phase("validation.prepare_baseline", source.display().to_string());
    let original = snapshot_in(source, &cache)?;
    ensure!(original.manifest.archive_sha256 == source_sha256, "source changed during baseline preparation");
    let mut archive = ZipArchive::new(File::open(&original.path)?)?;
    let mut xml = Vec::new(); archive.by_name("xl/workbook.xml")?.read_to_end(&mut xml)?;
    let cleaned = excel_shrink_core::workbook::clean_workbook(&xml)?;
    ensure!(excel_shrink_core::workbook::clean_workbook(&cleaned)? == cleaned, "workbook repair is not idempotent");
    let changed = xml != cleaned;
    let path = directory.join(source.file_name().context("input has no filename")?);
    let pending = unique_path(&directory, "baseline")?;
    if !changed { fs::copy(&original.path, &pending)?; }
    else {
        let mut writer = ZipWriter::new(File::create(&pending)?);
        for index in 0..archive.len() {
            let member = archive.by_index(index)?;
            if member.name() == "xl/workbook.xml" {
                writer.start_file("xl/workbook.xml", zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated))?;
                writer.write_all(&cleaned)?;
            } else { writer.raw_copy_file(member)?; }
        }
        writer.finish()?.sync_all()?;
    }
    atomic_replace(&pending, &path)?;
    let repaired = manifest_in(&path, &cache)?;
    let changes = changes_between(&original.manifest, &repaired, |name| name == "xl/workbook.xml");
    ensure!(changes.iter().all(|c| c.allowed), "baseline repair changed non-workbook parts");
    let result = PreparedBaseline { source_path, source_sha256, repair_key,
        snapshot: Snapshot { path, manifest: repaired }, workbook_xml_changed: changed };
    write_record(&record, &result)?;
    tracing::info!("[baseline] prepared {} workbook_xml_changed={changed}", result.snapshot.path.display());
    Ok(result)
}

pub(crate) fn changes_between(before: &PackageManifest, after: &PackageManifest, allowed: impl Fn(&str) -> bool) -> Vec<PackageChange> {
    let names: BTreeSet<_> = before.parts.keys().chain(after.parts.keys()).collect();
    names.into_iter().filter(|name| {
        before.parts.get(*name).map(|p| (&p.sha256, p.bytes)) != after.parts.get(*name).map(|p| (&p.sha256, p.bytes))
    }).map(|name| PackageChange { part: name.clone(), allowed: allowed(name),
        before: before.parts.get(name).cloned(), after: after.parts.get(name).cloned(), diff_directory: None }).collect()
}
pub(crate) fn assert_shrink_invariants(baseline: &Path, shrunk: &Path, report_dir: &Path) -> Result<Vec<PackageChange>> {
    let left = manifest(baseline)?; let right = manifest(shrunk)?;
    // Add/remove of worksheets is never allowed. calcChain contents may change,
    // but removing the part/relationship is not silently allowed here either.
    let mut changes = changes_between(&left, &right, |p| (is_sheet(p) || p == "xl/calcChain.xml")
        && left.parts.contains_key(p) && right.parts.contains_key(p));
    for change in &mut changes {
        if !change.allowed {
            let directory = dump_part_difference(baseline, shrunk, change, report_dir)?;
            tracing::error!("UNEXPECTED PACKAGE CHANGE: {} diff={}", change.part, directory.join("diff.txt").display());
            change.diff_directory = Some(directory);
        }
    }
    fs::create_dir_all(report_dir)?;
    atomic_write(&report_dir.join("package-changes.json"), &serde_json::to_vec_pretty(&changes)?)?;
    ensure!(changes.iter().all(|c| c.allowed), "unexpected non-worksheet ZIP change; see {}", report_dir.display());
    Ok(changes)
}
pub(crate) fn assert_snapshot_unchanged(before: &Snapshot, current: &Path, report_dir: &Path) -> Result<()> {
    let after = manifest(current)?;
    let mut changes = changes_between(&before.manifest, &after, |_| false);
    let archive_changed = before.manifest.archive_sha256 != after.archive_sha256;
    if archive_changed {
        ensure!(hash_file(&before.path)? == before.manifest.archive_sha256,
            "validation input and preserved snapshot were modified; original bytes are no longer available for a reliable diff");
        for change in &mut changes {
            change.diff_directory = Some(dump_part_difference(&before.path, current, change, report_dir)?);
            tracing::error!("VALIDATION INPUT MUTATED: {} report={}", change.part, report_dir.display());
        }
        atomic_write(&report_dir.join("mutations.json"), &serde_json::to_vec_pretty(&serde_json::json!({
            "before_archive": before.manifest.archive_sha256, "after_archive": after.archive_sha256,
            "container_only": changes.is_empty(), "changes": changes,
            "before": before.path, "after": current,
        }))?)?;
        if changes.is_empty() { tracing::error!("VALIDATION CONTAINER MUTATED with unchanged ZIP member contents; report={}", report_dir.display()); }
    }
    ensure!(!archive_changed, "validation stage modified its input archive; see {}", report_dir.display());
    Ok(())
}

/// A stable logical key links different content hashes from successive runs.
/// Baselines and objects are real copies, never writable hardlinks to corpus data.
pub(crate) fn record_output(source: &Path, profile: &str, output: &Path, report_dir: &Path) -> Result<()> {
    let logical = sha256(format!("{}\0{profile}", fs::canonicalize(source)?.display()).as_bytes());
    let history = root()?.join("history").join(logical);
    let latest = history.join("latest.json");
    let current = snapshot(output)?;
    let prior = read_record::<Snapshot>(&latest);
    atomic_write(&report_dir.join("output-history.json"), &serde_json::to_vec_pretty(&serde_json::json!({
        "source": source, "profile": profile, "previous": prior, "current": current,
    }))?)?;
    if let Some(previous) = prior {
        write_record(&history.join("previous.json"), &previous)?;
        if previous.manifest.package_sha256 != current.manifest.package_sha256 {
            let changes = changes_between(&previous.manifest, &current.manifest, |_| false);
            atomic_write(&report_dir.join("previous-run-package-changes.json"), &serde_json::to_vec_pretty(&changes)?)?;
            if std::env::var("EXCEL_SHRINK_PACKAGE_DIFF").is_ok_and(|v| v == "1") && hash_file(&previous.path).ok().as_deref() == Some(previous.manifest.archive_sha256.as_str()) {
                for change in &changes { dump_part_difference(&previous.path, &current.path, change, &report_dir.join("previous-run"))?; }
            }
            tracing::info!("[package-history] {} content changes; previous={} current={} report={}",
                changes.len(), previous.path.display(), current.path.display(), report_dir.display());
        }
    }
    write_record(&latest, &current)
}

fn member(path: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let result = match zip.by_name(name) {
        Ok(mut file) => { let mut bytes = Vec::new(); file.read_to_end(&mut bytes)?; Ok(Some(bytes)) },
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(error.into()),
    };
    result
}
fn dump_part_difference(before: &Path, after: &Path, change: &PackageChange, root: &Path) -> Result<PathBuf> {
    let safe: String = change.part.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(100).collect();
    let directory = root.join(format!("{safe}-{}", &sha256(change.part.as_bytes())[..12]));
    fs::create_dir_all(&directory)?;
    let a = member(before, &change.part)?; let b = member(after, &change.part)?;
    if let Some(bytes) = &a { atomic_write(&directory.join("before.bin"), bytes)?; }
    if let Some(bytes) = &b { atomic_write(&directory.join("after.bin"), bytes)?; }
    atomic_write(&directory.join("metadata.json"), &serde_json::to_vec_pretty(change)?)?;
    let diff = diagnostic_diff(a.as_deref().unwrap_or(&[]), b.as_deref().unwrap_or(&[]));
    atomic_write(&directory.join("diff.txt"), diff.as_bytes())?;
    Ok(directory)
}
fn diagnostic_diff(a: &[u8], b: &[u8]) -> String {
    let first = a.iter().zip(b).position(|(a, b)| a != b).unwrap_or(a.len().min(b.len()));
    let header = format!("--- before\n+++ after\n# first different byte: {first}; before={} bytes; after={} bytes\n", a.len(), b.len());
    if a.len().max(b.len()) > 16 * 1024 * 1024 || std::str::from_utf8(a).is_err() || std::str::from_utf8(b).is_err() {
        let lo = first.saturating_sub(128);
        return format!("{header}# binary/large part: bounded byte context, full originals in before.bin / after.bin\n-{:?}\n+{:?}\n",
            &a[lo.min(a.len())..(first + 256).min(a.len())], &b[lo.min(b.len())..(first + 256).min(b.len())]);
    }
    // Diagnostic formatting only; original bytes are never rewritten.
    let a = std::str::from_utf8(a).unwrap().replace("><", ">\n<");
    let b = std::str::from_utf8(b).unwrap().replace("><", ">\n<");
    let aa: Vec<_> = a.lines().collect(); let bb: Vec<_> = b.lines().collect();
    let prefix = aa.iter().zip(&bb).take_while(|(a, b)| a == b).count();
    let suffix = aa[prefix..].iter().rev().zip(bb[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let start = prefix.saturating_sub(3);
    let aend = (aa.len() - suffix + 3).min(aa.len()); let bend = (bb.len() - suffix + 3).min(bb.len());
    let mut result = format!("{header}@@ -{},{} +{},{} @@\n", start + 1, aend - start, start + 1, bend - start);
    for (prefix, lines) in [('-', &aa[start..aend]), ('+', &bb[start..bend])] {
        for line in lines {
            if result.len() > 512 * 1024 { result.push_str("\n# DIFF TRUNCATED; full raw parts preserved.\n"); return result; }
            result.push(prefix); result.push_str(line); result.push('\n');
        }
    }
    result
}

/// Log expected serialization changes without feeding the saved file to rendering.
pub(crate) fn record_stage_change(before: &Snapshot, after: &Path, report_dir: &Path) -> Result<()> {
    let after_snapshot = snapshot(after)?;
    let changes = changes_between(&before.manifest, &after_snapshot.manifest, |_| true);
    atomic_write(&report_dir.join("package-changes.json"), &serde_json::to_vec_pretty(&changes)?)?;
    write_record(&report_dir.join("before.json"), before)?;
    write_record(&report_dir.join("after.json"), &after_snapshot)?;
    if std::env::var("EXCEL_SHRINK_PACKAGE_DIFF").is_ok_and(|v| v == "1") {
        for change in &changes { dump_part_difference(&before.path, after, change, report_dir)?; }
    }
    tracing::info!("[package-stage] {} changed parts; snapshots/diff report={}", changes.len(), report_dir.display());
    Ok(())
}

#[cfg(test)]
#[path = "artifact_cache_tests.rs"]
mod tests;
