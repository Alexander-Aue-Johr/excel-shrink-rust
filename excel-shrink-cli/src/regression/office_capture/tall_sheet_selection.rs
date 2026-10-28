//! Shorten only unusually tall sheets. Omitted bands are an explicit sampling
//! decision, never a pixel-equality certificate. Saved recalc copies are inputs
//! to selection only; capture clients keep the unmodified original/profile files.
use super::*;
use crate::regression::{excel_automation, xlsx_value_compare};

const MAX_CAPTURE_TILES: usize = 4096;
const MIB: u64 = 1024 * 1024;

#[derive(Debug)]
struct CrcEqualSheet {
    recalc_equal: Option<bool>,
    changed_profiles: Vec<String>,
}

fn member_signature(path: &Path, member: &str) -> Result<(u64, u32)> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let entry = archive.by_name(member)?;
    Ok((entry.size(), entry.crc32()))
}

fn crc_equal_sheets(
    original: &Path,
    variants: &BTreeMap<String, PathBuf>,
    hints: &Value,
    threshold_mib: usize,
) -> Result<BTreeMap<usize, CrcEqualSheet>> {
    let paths = tall_sheet_analysis::sheet_paths(original)?;
    let mut result = BTreeMap::new();
    for (index, member) in paths {
        if variants.is_empty() || !variants.keys().all(|profile| {
            let evidence = &hints["shrink_identity"][profile.as_str()]["sheets"][member.as_str()];
            evidence["crc_length_equal"] == true
                && evidence["original_uncompressed_bytes"]
                    .as_u64()
                    .is_some_and(|bytes| bytes >= threshold_mib as u64 * MIB)
        }) {
            continue;
        }

        let baseline = hints["original"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_file());
        let baseline_signature = baseline
            .as_deref()
            .and_then(|path| member_signature(path, &member).ok());
        let mut changed_profiles = Vec::new();
        let mut unavailable = baseline_signature.is_none();
        for profile in variants.keys() {
            let saved = hints["variants"][profile.as_str()]
                .as_str()
                .map(PathBuf::from)
                .filter(|path| path.is_file());
            let saved_signature = saved
                .as_deref()
                .and_then(|path| member_signature(path, &member).ok());
            match baseline_signature.zip(saved_signature) {
                Some((before, after)) if before != after => changed_profiles.push(profile.clone()),
                Some(_) => {}
                None => unavailable = true,
            }
        }
        let recalc_equal = if !changed_profiles.is_empty() {
            Some(false)
        } else if unavailable {
            None
        } else {
            Some(true)
        };
        result.insert(index, CrcEqualSheet { recalc_equal, changed_profiles });
    }
    Ok(result)
}

fn setting(live: Option<&live::LiveCapture>, key: &str, env: &str, default: u64) -> usize {
    live.and_then(|l| l.tall_sheet_setting(key))
        .or_else(|| std::env::var(env).ok()?.parse().ok())
        .filter(|n| (1..=i32::MAX as u64).contains(n))
        .unwrap_or(default) as usize
}

fn overlaps(a: [u32; 2], b: [u32; 2]) -> bool {
    a[0] < b[1] && b[0] < a[1]
}

fn selected_indices(
    tiles: &[&Tile],
    rows: &[[u32; 2]],
    threshold: usize,
    edges: usize,
    fallback: bool,
) -> Result<BTreeSet<usize>> {
    let selected: BTreeSet<_> = tiles
        .iter()
        .enumerate()
        .filter(|(i, tile)| {
            tiles.len() <= threshold
                || fallback
                || *i < edges
                || *i >= tiles.len().saturating_sub(edges)
                || rows.iter().any(|span| overlaps(tile.core_rows, *span))
        })
        .map(|(_, tile)| tile.tile_index)
        .collect();
    // Match the worker's actual-capture limit after selection. Large metadata
    // plans are allowed, but conservative full-capture fallbacks stay bounded.
    ensure!(
        selected.len() <= MAX_CAPTURE_TILES,
        "IMAGE_BUDGET: selected captures exceed the {MAX_CAPTURE_TILES}-tile limit"
    );
    Ok(selected)
}

fn merge_rows(rows: &mut Vec<[u32; 2]>) {
    rows.sort_unstable();
    let mut merged: Vec<[u32; 2]> = Vec::new();
    for span in rows.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| span[0] <= last[1]) {
            last[1] = last[1].max(span[1]);
        } else {
            merged.push(span);
        }
    }
    *rows = merged;
}

// Finish isolated Excel recalculations before resident add-ins are opened. A
// row-count upper bound is deliberately conservative: actual pixel tile cuts
// are available only after Excel has measured the sheet. Existing study checks
// have already saved the profile copies and often the original reference too.
pub(super) fn prepare_recalc_inputs(
    original: &Path,
    variants: &[(&str, PathBuf)],
    directory: &Path,
    plan: &PairedPlan,
    live: Option<&live::LiveCapture>,
) -> Result<()> {
    let threshold = setting(
        live,
        "tall_sheet_threshold",
        "EXCEL_SHRINK_TALL_SHEET_THRESHOLD",
        40,
    );
    if plan
        .sheets
        .iter()
        .all(|s| (s.bottom_row as usize) < threshold)
    {
        return Ok(());
    }
    let path = directory.join("recalc-inputs.json");
    let mut hints: Value = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({"variants":{}}));
    let root = directory.join("selection-recalc");
    let available = |v: &Value| v.as_str().is_some_and(|p| Path::new(p).is_file());
    crate::regression::archive::work_state::event(
        "validating_values",
        "selection_recalc_preflight",
        "Preparing saved recalculation references before opening capture clients",
        false,
        Some("isolated Excel recalculation"),
        json!({}),
    );
    let result = (|| -> Result<()> {
        if !available(&hints["original"]) {
            let reference = excel_automation::baseline_recalculation_reference(
                original,
                &root,
                &root.join("automation"),
            )?;
            hints["original"] = json!(reference.workbook.path);
        }
        for (name, variant) in variants {
            if !available(&hints["variants"][*name]) {
                let saved = excel_automation::save_recalculated_copy(
                    variant,
                    &root.join(format!("{name}-recalculated.xlsx")),
                    &root.join(name),
                )?;
                hints["variants"][*name] = json!(saved.path);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        // A selection aid failing must never turn missing evidence into a pass.
        hints["error"] = json!(format!("{error:#}"));
    }
    publish(&path, &hints)
}

fn value_rows(
    left: &Path,
    right: &Path,
    wanted: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, Vec<[u32; 2]>>> {
    let paths = tall_sheet_analysis::sheet_paths(left)?;
    let files: std::collections::HashSet<_> = paths
        .iter()
        .filter(|(index, _)| wanted.contains(index))
        .map(|(_, path)| path.clone())
        .collect();
    let index: BTreeMap<_, _> = paths
        .iter()
        .map(|(index, path)| (path.as_str(), *index))
        .collect();
    let changed = xlsx_value_compare::compare_workbook_changed_rows(left, right, &files)?;
    let mut rows = BTreeMap::<usize, Vec<[u32; 2]>>::new();
    for (file, spans) in changed {
        let sheet = *index
            .get(file.as_str())
            .context("recalc changed unknown sheet")?;
        rows.insert(sheet, spans);
    }
    for spans in rows.values_mut() {
        merge_rows(spans);
    }
    Ok(rows)
}

impl ResidentCapture {
    fn recalc_selection(
        &self,
        wanted: &BTreeSet<usize>,
    ) -> Result<(BTreeMap<usize, Vec<[u32; 2]>>, Value)> {
        let hints: Value = fs::read(self.directory.join("recalc-inputs.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        ensure!(
            hints["error"].is_null(),
            "recalc preflight failed: {}",
            hints["error"]
        );
        let baseline = hints["original"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_file());
        let baseline = baseline.context("original recalc preflight unavailable")?;
        let original_rows = value_rows(&self.original, &baseline, wanted)?;
        let mut selected = original_rows.clone();
        let mut copies =
            vec![json!({"kind":"original", "sha256":artifact_cache::hash_file(&baseline)?})];
        for (name, variant) in &self.variants {
            let saved = hints["variants"][name]
                .as_str()
                .map(PathBuf::from)
                .filter(|p| p.is_file());
            let saved = saved.context("variant recalc preflight unavailable")?;
            // Include both changed caches and the paired recalculated values.
            // Original drift is recorded, never subtracted from this union.
            for changes in [
                value_rows(variant, &saved, wanted)?,
                value_rows(&baseline, &saved, wanted)?,
            ] {
                for (sheet, spans) in changes {
                    selected.entry(sheet).or_default().extend(spans);
                }
            }
            copies.push(json!({"kind":name, "sha256":artifact_cache::hash_file(&saved)?}));
        }
        for spans in selected.values_mut() {
            merge_rows(spans);
        }
        Ok((
            selected,
            json!({"status":"complete", "purpose":"selection only; screenshots use unsaved input workbooks",
            "original_changed_rows":original_rows, "saved_copy_hashes":copies}),
        ))
    }

    pub(super) fn select_tall_sheet_captures(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        let threshold = setting(
            self.live.as_ref(),
            "tall_sheet_threshold",
            "EXCEL_SHRINK_TALL_SHEET_THRESHOLD",
            40,
        );
        let edges = setting(
            self.live.as_ref(),
            "tall_sheet_edge_tiles",
            "EXCEL_SHRINK_TALL_SHEET_EDGE_TILES",
            2,
        );
        let crc_equal_threshold_mib = setting(
            self.live.as_ref(),
            "crc_equal_sheet_threshold_mib",
            "EXCEL_SHRINK_CRC_EQUAL_SHEET_THRESHOLD_MIB",
            256,
        );
        let hints: Value = fs::read(self.directory.join("recalc-inputs.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let force_full_scan = hints["full_scan_deferred"] == true;
        let crc_equal = crc_equal_sheets(
            &self.original,
            &self.variants,
            &hints,
            crc_equal_threshold_mib,
        )?;
        let original = self.snapshots["original"].clone();
        let mut by_sheet = BTreeMap::<usize, Vec<&Tile>>::new();
        for tile in &original.tiles {
            by_sheet.entry(tile.sheet_index).or_default().push(tile);
        }
        let wanted: BTreeSet<_> = by_sheet
            .iter()
            .filter(|(_, tiles)| tiles.len() > threshold)
            .filter(|(index, _)| !crc_equal.contains_key(index))
            .map(|(index, _)| *index)
            .collect();
        let mut indicators = BTreeMap::new();
        let mut recalc = json!({"status":"not required: no tall sheets"});
        let mut failure = None;
        if !wanted.is_empty() {
            crate::regression::archive::work_state::event("selecting_captures", "tall_sheet_analysis",
                "Checking XML, saved recalculations and measured row heights before shortening tall sheets",
                false, Some("capture selection analysis"), json!({"sheets":wanted}));
            match tall_sheet_analysis::analyze(
                &self.original,
                &self.variants.values().cloned().collect::<Vec<_>>(),
                &wanted,
            ) {
                Ok(result) => indicators = result,
                Err(error) => failure = Some(format!("XML analysis unavailable: {error:#}")),
            }
            if failure.is_none()
                && wanted.iter().any(|s| {
                    indicators
                        .get(s)
                        .is_some_and(|i| i.full_capture_reason.is_none())
                })
            {
                match self.recalc_selection(&wanted) {
                    Ok((changes, metadata)) => {
                        recalc = metadata;
                        for (sheet, rows) in changes {
                            if let Some(indicator) = indicators.get_mut(&sheet) {
                                if !rows.is_empty() {
                                    indicator
                                        .reasons
                                        .push("recalculated cached cell values changed".into());
                                }
                                indicator.rows.extend(rows);
                            }
                        }
                    }
                    Err(error) => {
                        failure = Some(format!("Recalc selection unavailable: {error:#}"));
                        recalc = json!({"status":"failed", "error":failure});
                    }
                }
            } else {
                recalc = json!({"status":"not required: XML already requires full capture"});
            }
            let extents: Vec<_> = self
                .plans
                .values()
                .next()
                .unwrap()
                .sheets
                .iter()
                .map(|s| SheetExtent {
                    index: s.sheet_index,
                    name: s.sheet_name.clone(),
                    last_row: s.bottom_row + 1,
                    last_checked_row: s.row_height_bottom_row + 1,
                })
                .collect();
            for client in clients {
                for change in row_heights::compare_row_heights(
                    &original.heights,
                    &self.snapshots[client].heights,
                    &extents,
                )? {
                    if let Some(indicator) = indicators.get_mut(&change.sheet_index) {
                        indicator.rows.push([change.row - 1, change.end_row]);
                        indicator
                            .reasons
                            .push("measured Excel row heights changed".into());
                    }
                }
            }
        }
        let mut keep = BTreeSet::new();
        for (sheet, tiles) in by_sheet {
            let indicator = indicators.get_mut(&sheet);
            let crc_sample = crc_equal.get(&sheet);
            let full_deferred_scan = force_full_scan && crc_sample.is_some();
            let (rows, mut reasons, fallback) = if wanted.contains(&sheet) {
                match indicator {
                    Some(i) => {
                        merge_rows(&mut i.rows);
                        (
                            i.rows.clone(),
                            i.reasons.clone(),
                            failure.clone().or_else(|| i.full_capture_reason.clone()),
                        )
                    }
                    None => (
                        vec![],
                        vec![],
                        Some(
                            failure
                                .clone()
                                .unwrap_or_else(|| "missing XML analysis".into()),
                        ),
                    ),
                }
            } else {
                (vec![], vec![], None)
            };
            if let Some(sample) = crc_sample {
                reasons.push(if full_deferred_scan {
                    "worksheet length and CRC-32 matched after shrink"
                } else {
                    "worksheet length and CRC-32 matched after shrink; sampling edges first"
                }.into());
                if sample.recalc_equal == Some(false) {
                    reasons.push("recalculated worksheet length or CRC-32 changed; full scan deferred".into());
                } else if sample.recalc_equal.is_none() {
                    reasons.push("recalculated worksheet identity unavailable; full scan deferred".into());
                }
            }
            if full_deferred_scan {
                reasons.push("manually resumed deferred full scan".into());
            }
            reasons.sort();
            reasons.dedup();
            let selected = selected_indices(
                &tiles,
                &rows,
                if crc_sample.is_some() { 0 } else { threshold },
                edges,
                fallback.is_some() || full_deferred_scan,
            )
                .with_context(|| format!("select captures for sheet {sheet}"))?;
            keep.extend(selected.iter().map(|tile| (sheet, *tile)));
            let mut gaps = Vec::<Value>::new();
            for tile in tiles.iter().filter(|t| !selected.contains(&t.tile_index)) {
                if let Some(last) = gaps.last_mut().filter(|last| {
                    last["last_tile_index"].as_u64() == Some(tile.tile_index as u64 - 1)
                }) {
                    last["last_tile_index"] = json!(tile.tile_index);
                    last["core_rows"][1] = json!(tile.core_rows[1]);
                    last["height"] =
                        json!(tile.y + tile.height - last["y"].as_u64().unwrap() as u32);
                    last["count"] = json!(last["count"].as_u64().unwrap() + 1);
                } else {
                    gaps.push(json!({"first_tile_index":tile.tile_index,"last_tile_index":tile.tile_index,
                        "core_rows":tile.core_rows,"y":tile.y,"height":tile.height,"count":1,
                        "reasons":["Outside top/bottom samples and XML, formatting, drawing, recalculation and measured-height candidate ranges; not visually checked"]}));
                }
            }
            let mut sheet_recalc = recalc.clone();
            if let Some(map) = sheet_recalc["original_changed_rows"].as_object() {
                let rows = map
                    .get(&sheet.to_string())
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                sheet_recalc["original_changed_rows"] = rows;
            }
            self.capture_coverage.insert(sheet, json!({"policy":if full_deferred_scan {"deferred-full-scan-v1"} else if crc_sample.is_some() {"crc-equal-edge-first-v1"} else {"tall-sheet-v1"}, "total_tiles":tiles.len(),
                "captured_tiles":selected.len(),"omitted_tiles":tiles.len()-selected.len(),"threshold":threshold,"edge_tiles":edges,
                "omitted_ranges":gaps,"selection_reasons":reasons,"fallback_reason":fallback,
                "crc_length_equal_after_shrink":crc_sample.is_some(),
                "crc_length_equal_after_recalc":crc_sample.and_then(|sample| sample.recalc_equal),
                "recalc_identity_changed":crc_sample.is_some_and(|sample| sample.recalc_equal == Some(false)),
                "recalc_changed_profiles":crc_sample.map(|sample| &sample.changed_profiles),
                "deferred_full_scan":crc_sample.is_some() && selected.len() < tiles.len(),
                "checks_complete":selected.len() == tiles.len(),
                "crc_equal_threshold_mib":crc_equal_threshold_mib,
                "recalc":if wanted.contains(&sheet) {sheet_recalc} else {json!({"status":"not required"})}}));
        }
        // Publish the full geometry once, then mark the intentionally absent tiles.
        if let Some(live) = &mut self.live {
            live.selected_plan(
                &original,
                &self.snapshots,
                &self.aliases,
                &self.capture_coverage,
                &keep,
            )?;
        }
        for snapshot in self.snapshots.values_mut() {
            snapshot
                .tiles
                .retain(|t| keep.contains(&(t.sheet_index, t.tile_index)));
        }
        publish(
            &self.directory.join("capture-coverage.json"),
            &json!(self.capture_coverage),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn identity_workbook(path: &Path, value: &str) {
        let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, xml) in [
            (
                "xl/workbook.xml",
                "<workbook><sheets><sheet name=\"Data\" r:id=\"r1\"/></sheets></workbook>",
            ),
            (
                "xl/_rels/workbook.xml.rels",
                "<Relationships><Relationship Id=\"r1\" Type=\"office/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
            ),
            ("xl/worksheets/sheet1.xml", value),
        ] {
            writer.start_file(name, zip::write::FileOptions::default()).unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn crc_equal_sheet_records_recalc_drift_without_reading_sheet_xml() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("crc_equal_selection_{unique}"));
        fs::create_dir_all(&root).unwrap();
        let original = root.join("original.xlsx");
        let variant = root.join("variant.xlsx");
        let baseline = root.join("baseline.xlsx");
        let saved = root.join("saved.xlsx");
        identity_workbook(&original, "<worksheet><v>1</v></worksheet>");
        identity_workbook(&variant, "<worksheet><v>1</v></worksheet>");
        identity_workbook(&baseline, "<worksheet><v>1</v></worksheet>");
        identity_workbook(&saved, "<worksheet><v>2</v></worksheet>");
        let variants = BTreeMap::from([("default".to_string(), variant)]);
        let hints = json!({"original":baseline,"shrink_identity":{"default":{"sheets":{
            "xl/worksheets/sheet1.xml":{"crc_length_equal":true,"original_uncompressed_bytes":31}
        }}},"variants":{"default":saved}});

        let result = crc_equal_sheets(&original, &variants, &hints, 0).unwrap();
        assert_eq!(result[&1].recalc_equal, Some(false));
        assert_eq!(result[&1].changed_profiles, ["default"]);
        identity_workbook(&baseline, "<worksheet><v>2</v></worksheet>");
        let recalc_equal = crc_equal_sheets(&original, &variants, &hints, 0).unwrap();
        assert_eq!(recalc_equal[&1].recalc_equal, Some(true));
        assert!(crc_equal_sheets(&original, &variants, &hints, 1)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    fn tile(index: usize) -> Tile {
        Tile {
            sheet_index: 1,
            sheet_name: "CSV".into(),
            tile_index: index,
            range: format!("A{}:B{}", index * 100 + 1, index * 100 + 100),
            core_rows: [index as u32 * 100, index as u32 * 100 + 100],
            capture_right_column: 1,
            width: 200,
            height: 100,
            x: 0,
            y: index as u32 * 100,
            capture_pixels: 20000,
            pixel_budget: 0,
            capture_budget_fallback: None,
            sha256: "0".repeat(64),
        }
    }
    #[test]
    fn only_tall_sheets_are_shortened_and_boundary_is_strict() {
        let tiles: Vec<_> = (0..10).map(tile).collect();
        let refs: Vec<_> = tiles.iter().collect();
        assert_eq!(
            selected_indices(&refs, &[], 10, 2, false).unwrap().len(),
            10
        );
        assert_eq!(
            selected_indices(&refs, &[], 9, 2, false).unwrap(),
            BTreeSet::from([0, 1, 8, 9])
        );
    }
    #[test]
    fn middle_indicators_include_each_intersected_band_and_fallback_keeps_all() {
        let tiles: Vec<_> = (0..10).map(tile).collect();
        let refs: Vec<_> = tiles.iter().collect();
        assert_eq!(
            selected_indices(&refs, &[[399, 501]], 5, 1, false).unwrap(),
            BTreeSet::from([0, 3, 4, 5, 9])
        );
        assert_eq!(selected_indices(&refs, &[], 5, 1, true).unwrap().len(), 10);
        assert_eq!(
            selected_indices(&refs, &[], 5, 20, false).unwrap().len(),
            10
        );
    }
    #[test]
    fn large_plans_are_shortened_before_the_capture_budget_applies() {
        let tiles: Vec<_> = (0..5000).map(tile).collect();
        let refs: Vec<_> = tiles.iter().collect();
        assert_eq!(
            selected_indices(&refs, &[], 40, 2, false).unwrap(),
            BTreeSet::from([0, 1, 4998, 4999])
        );
        for selected in [
            selected_indices(&refs, &[], 40, 2, true),
            selected_indices(&refs, &[[0, 500000]], 40, 2, false),
            selected_indices(&refs, &[], 6000, 2, false),
        ] {
            assert!(selected.unwrap_err().to_string().contains("IMAGE_BUDGET"));
        }
    }
    #[test]
    fn merging_does_not_fill_unobserved_gaps() {
        let mut rows = vec![[100, 110], [0, 5], [4, 10], [110, 115]];
        merge_rows(&mut rows);
        assert_eq!(rows, vec![[0, 10], [100, 115]]);
    }
}
