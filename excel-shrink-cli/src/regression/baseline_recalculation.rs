// Included by excel_automation.rs. Screenshots never consume these saved copies.
pub(super) struct BaselineRecalculationReference {
    pub workbook: ExcelSavedWorkbook,
    pub cache_hit: bool,
    pub cacheable: bool,
    pub kind: String,
}
#[derive(Serialize, Deserialize)]
struct StoredCalculationReference {
    key: String,
    snapshot: super::artifact_cache::Snapshot,
}
pub(super) fn baseline_recalculation_reference(
    input: &Path, excel_dir: &Path, work_dir: &Path,
) -> anyhow::Result<BaselineRecalculationReference> {
    use super::artifact_cache as cache;
    let manifest = cache::manifest(input)?;
    let certificate = manifest.parts.values().all(|p| p.recalc_cacheable);
    let cacheable = certificate && !std::env::var("EXCEL_SHRINK_RECALC_REFERENCE_CACHE").is_ok_and(|v| v == "0");
    // Includes the entire package, Excel/Office environment and the exact scripts
    // and certificate implementation, never only the current worksheet's XML.
    let environment = recalculation_environment_key(work_dir)?;
    let key = cache::sha256(format!("recalc-reference-v10/{}/{}/{}/{}/{}", manifest.package_sha256,
        environment, cache::sha256(RECALCULATE_AND_SAVE_SCRIPT.as_bytes()),
        cache::sha256(include_bytes!("artifact_cache.rs")), cache::sha256(include_bytes!("baseline_recalculation.rs"))).as_bytes());
    let record_path = cache::root()?.join("calculation-references").join(format!("{key}.json"));
    if cacheable {
        if let Some(saved) = cache::read_record::<StoredCalculationReference>(&record_path) {
            if saved.key == key && cache::hash_file(&saved.snapshot.path).ok().as_deref() == Some(saved.snapshot.manifest.archive_sha256.as_str()) {
                tracing::info!("[recalc] baseline reference cache hit; original workbook will not be opened");
                cache::atomic_write(&excel_dir.join("baseline-reference.json"), &serde_json::to_vec_pretty(&saved)?)?;
                return Ok(BaselineRecalculationReference { workbook: ExcelSavedWorkbook { path: saved.snapshot.path },
                    cache_hit: true, cacheable, kind: "recalculated-cached-baseline".into() });
            }
        }
    }
    tracing::info!("[recalc] recalculating baseline; reusable_reference={cacheable}");
    let output = excel_dir.join("original-recalculated").join(input.file_name().unwrap_or(OsStr::new("baseline.xlsx")));
    let saved = save_recalculated_copy(input, &output, &work_dir.join("baseline"))?;
    let snapshot = cache::snapshot(&saved.path)?;
    let reference = StoredCalculationReference { key, snapshot };
    if cacheable { cache::write_record(&record_path, &reference)?; }
    cache::atomic_write(&excel_dir.join("baseline-reference.json"), &serde_json::to_vec_pretty(&reference)?)?;
    Ok(BaselineRecalculationReference { workbook: ExcelSavedWorkbook { path: reference.snapshot.path },
        cache_hit: false, cacheable, kind: if cacheable { "recalculated-new-baseline" } else { "recalculated-uncacheable-baseline" }.into() })
}
