//! Bounded, cumulative counterfactuals on already-open temporary Excel copies.
use super::*;
use crate::regression::xlsx_value_compare::removed_diagnostic_cells;
use crate::regression::xml_views::{Kind, Reader, TextBytes};
use std::io::Read;

const MAX_DIAGNOSIS_TILES: usize = 256;
const MAX_RESTORED_CELLS: usize = 20_000;

#[derive(Clone)]
struct Evidence {
    snapshot: Snapshot,
    images: BTreeMap<(usize, String), PathBuf>,
}

fn selected_regions(original: &Snapshot, primary: &Value) -> Result<Snapshot> {
    let sheets = primary["sheets"]
        .as_array()
        .context("diagnosis primary sheets missing")?;
    let mut ranges = BTreeSet::new();
    let mut height_sheets = BTreeSet::new();
    for sheet in sheets {
        let index = sheet["index"]
            .as_u64()
            .context("diagnosis sheet index missing")? as usize;
        if sheet["row_heights"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            height_sheets.insert(index);
        }
        for difference in sheet["image_differences"].as_array().into_iter().flatten() {
            ranges.insert((
                index,
                difference["range"]
                    .as_str()
                    .context("diagnosis range missing")?
                    .to_owned(),
            ));
        }
    }
    let tiles: Vec<_> = original
        .tiles
        .iter()
        .filter(|tile| {
            ranges.contains(&(tile.sheet_index, tile.range.clone()))
                || height_sheets.contains(&tile.sheet_index)
        })
        .cloned()
        .collect();
    ensure!(
        !tiles.is_empty(),
        "no primary mismatch regions for diagnosis"
    );
    ensure!(
        tiles.len() <= MAX_DIAGNOSIS_TILES,
        "diagnosis exceeds the 256-tile replay budget"
    );
    Ok(Snapshot {
        command: original.command.clone(),
        heights: original.heights.clone(),
        tiles,
    })
}

fn sheet_descriptors(path: &Path) -> Result<BTreeMap<usize, (String, String, bool)>> {
    let paths = tall_sheet_analysis::sheet_paths(path)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut part = archive.by_name("xl/workbook.xml")?;
    ensure!(
        part.size() <= 4 * 1024 * 1024,
        "diagnosis workbook metadata budget exceeded"
    );
    let mut bytes = Vec::new();
    part.read_to_end(&mut bytes)?;
    let mut reader = Reader::<128>::new(&bytes)?;
    let mut index = 0;
    let mut sheets = BTreeMap::new();
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End || tag.local() != b"sheet" {
            continue;
        }
        index += 1;
        if let Some(file) = paths.get(&index) {
            let name = tag.attr(b"name")?.context("diagnosis sheet name missing")?;
            let name = String::from_utf8(TextBytes::new(name).collect::<Result<Vec<_>>>()?)?;
            let hidden = matches!(tag.attr(b"state")?, Some(b"hidden" | b"veryHidden"));
            sheets.insert(index, (name, file.clone(), hidden));
        }
    }
    Ok(sheets)
}

fn height_repairs(reference: &Snapshot, variant: &Snapshot) -> Vec<Value> {
    let mut last_rows = BTreeMap::<usize, u32>::new();
    for tile in &reference.tiles {
        last_rows
            .entry(tile.sheet_index)
            .and_modify(|end| *end = (*end).max(tile.core_rows[1]))
            .or_insert(tile.core_rows[1]);
    }
    let mut result = Vec::new();
    for sheet in &reference.heights {
        let Some(last) = last_rows.get(&sheet.sheet_index) else {
            continue;
        };
        let Some(other) = variant
            .heights
            .iter()
            .find(|item| item.sheet_index == sheet.sheet_index)
        else {
            continue;
        };
        let (mut a, mut b) = (0, 0);
        while a < sheet.runs.len() && b < other.runs.len() {
            let run = &sheet.runs[a];
            let peer = &other.runs[b];
            let first = run.start_row.max(peer.start_row);
            let end = run.end_row.min(peer.end_row).min(*last);
            if first <= end && run.height != peer.height {
                result.push(
                    json!({"sheet_index":sheet.sheet_index,"sheet_name":sheet.sheet_name,
                    "start_row":first,"end_row":end,"height":run.height}),
                );
            }
            if run.end_row <= peer.end_row {
                a += 1;
            }
            if peer.end_row <= run.end_row {
                b += 1;
            }
        }
    }
    result
}

fn compare_evidence(left: &Evidence, right: &Evidence, output: &Path) -> Result<Value> {
    ensure!(
        same_ranges(&left.snapshot, &right.snapshot),
        "diagnosis replay changed cell regions"
    );
    let mut comparisons = Vec::new();
    let mut count = 0;
    let mut pixels = 0u64;
    for tile in &left.snapshot.tiles {
        let key = (tile.sheet_index, tile.range.clone());
        let before = left
            .images
            .get(&key)
            .context("diagnosis reference PNG missing")?;
        let after = right
            .images
            .get(&key)
            .context("diagnosis replay PNG missing")?;
        let a = image::open(before)?.to_rgba8();
        let b = image::open(after)?.to_rgba8();
        let pair = PairedImageTile {
            sheet_index: tile.sheet_index,
            sheet_name: tile.sheet_name.clone(),
            range: tile.range.clone(),
            tile_index: tile.tile_index,
            original_image: before.clone(),
            shrunk_image: after.clone(),
        };
        let differences = visual_compare::compare_tile_images(&pair, &a, &b, output)?;
        let changed = !differences.is_empty();
        count += usize::from(changed);
        pixels += differences
            .iter()
            .map(|difference| difference.diff_pixels)
            .sum::<u64>();
        comparisons.push(json!({"sheet_index":tile.sheet_index,"range":tile.range,
            "changed":changed,"original_image":before,"shrunk_image":after,"image_differences":differences}));
    }
    Ok(json!({"same":count == 0,"changed_tiles":count,"diff_pixels":pixels,"tiles":comparisons}))
}

fn resolved_category(applied: &[String]) -> &str {
    if applied.len() == 1 {
        &applied[0]
    } else {
        "combined"
    }
}

impl ResidentCapture {
    fn diagnosis_command(&mut self, client: &str, mut body: Value) -> Result<Value> {
        body["kind"] = json!("diagnosis");
        let command = self.issue(client, body)?;
        let result = self.wait(client, &command)?;
        ensure!(
            result["diagnostic_error"].is_null(),
            "supplemental diagnosis failed: {}",
            result["diagnostic_error"]
        );
        Ok(result)
    }

    fn freeze_diagnosis_evidence(
        &self,
        client: &str,
        snapshot: Snapshot,
        selected: &Snapshot,
        output: &Path,
    ) -> Result<Evidence> {
        let physical = self.client(client)?;
        fs::create_dir_all(output)?;
        let mut images = BTreeMap::new();
        let mut tiles = Vec::new();
        for selected_tile in &selected.tiles {
            let tile = snapshot
                .tiles
                .iter()
                .find(|tile| {
                    tile.sheet_index == selected_tile.sheet_index
                        && tile.range == selected_tile.range
                })
                .context("diagnosis source region missing")?;
            let source = self
                .images
                .get(&(
                    physical.to_owned(),
                    snapshot.command.clone(),
                    tile.sheet_index,
                    tile.tile_index,
                ))
                .context("diagnosis source image was not retained")?;
            let target = output.join(format!(
                "s{}-t{}.png",
                selected_tile.sheet_index, selected_tile.tile_index
            ));
            fs::copy(source, &target)?;
            images.insert((tile.sheet_index, tile.range.clone()), target);
            let mut retained = tile.clone();
            retained.tile_index = selected_tile.tile_index;
            tiles.push(retained);
        }
        let frozen = Evidence {
            snapshot: Snapshot { tiles, ..snapshot },
            images,
        };
        fs::write(
            output.join("capture.json"),
            serde_json::to_vec_pretty(&json!({
                "tiles":frozen.snapshot.tiles,"heights":frozen.snapshot.heights,"images":frozen.images.iter()
                    .map(|((sheet,range),path)|json!({"sheet_index":sheet,"range":range,"path":path})).collect::<Vec<_>>()
            }))?,
        )?;
        Ok(frozen)
    }

    fn replay_diagnosis(
        &mut self,
        profile: &str,
        client: &str,
        selected: &Snapshot,
        output: &Path,
    ) -> Result<Evidence> {
        let plan = self
            .plans
            .get(profile)
            .context("diagnosis profile plan missing")?
            .clone();
        let mut configs = Vec::new();
        for sheet in &plan.sheets {
            let tiles: Vec<_> = selected
                .tiles
                .iter()
                .filter(|tile| tile.sheet_index == sheet.sheet_index)
                .collect();
            if tiles.is_empty() {
                continue;
            }
            let mut config = if client == "original" {
                sheet.original_config.clone()
            } else {
                sheet.shrunk_config.clone()
            };
            config["tile_rows"] =
                json!(tiles.iter().map(|tile| tile.core_rows).collect::<Vec<_>>());
            config["capture_end_column"] = json!(tiles[0].capture_right_column);
            config["capture_right_column"] = json!(tiles[0].capture_right_column);
            configs.push(config);
        }
        self.recapture(client, configs)?;
        let mut snapshot = self.snapshot(client)?;
        ensure!(
            same_ranges(selected, &snapshot),
            "diagnosis plan changed requested regions"
        );
        self.capture_target_images(client, &mut snapshot)?;
        self.snapshots
            .insert(self.client(client)?.to_owned(), snapshot.clone());
        self.freeze_diagnosis_evidence(client, snapshot, selected, output)
    }

    /// Primary evidence is immutable; every diagnostic capture has its own directory.
    pub(in crate::regression) fn diagnose_profile(
        &mut self,
        profile: &str,
        primary: &Value,
        output: &Path,
    ) -> Result<Value> {
        fs::create_dir_all(output)?;
        let mut report = json!({"version":1,"profile":profile,"status":"running","category":"unresolved",
            "method":"cumulative in-memory counterfactuals","scope":"primary mismatch tiles",
            "causal_claim":"resolution by accumulated interventions, not independent necessity",
            "steps":[],"applied_steps":[],"original_inputs_modified_on_disk":false,"pixel_shift_reconstruction_used":false});
        if let Err(error) = self.run_diagnosis(profile, primary, output, &mut report) {
            report["status"] = json!("unresolved");
            report["category"] = json!("unresolved");
            report["error"] = json!(format!("{error:#}"));
        }
        fs::write(
            output.join("diagnosis.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    }

    fn run_diagnosis(
        &mut self,
        profile: &str,
        primary: &Value,
        output: &Path,
        report: &mut Value,
    ) -> Result<()> {
        ensure!(
            self.sweep_mode,
            "targeted diagnosis requires experimental sweep mode"
        );
        ensure!(
            self.client(profile)? != self.client("original")?,
            "cannot mutate a shared original capture client"
        );
        let original = self
            .compared_originals
            .get(profile)
            .context("diagnosis primary capture missing")?
            .clone();
        let selected = selected_regions(&original, primary)?;
        report["checked_ranges"] = json!(
            selected
                .tiles
                .iter()
                .map(|tile| json!({"sheet_index":tile.sheet_index,"range":tile.range}))
                .collect::<Vec<_>>()
        );
        report["source_original_sha256"] = json!(artifact_cache::hash_file(&self.original)?);
        report["source_variant_sha256"] = json!(artifact_cache::hash_file(
            self.variants
                .get(profile)
                .context("diagnosis variant path missing")?
        )?);
        let pristine_original = self.freeze_diagnosis_evidence(
            "original",
            original,
            &selected,
            &output.join("pristine-original"),
        )?;
        let pristine_snapshot = self.snapshot(profile)?;
        let pristine = self.freeze_diagnosis_evidence(
            profile,
            pristine_snapshot,
            &selected,
            &output.join("pristine"),
        )?;
        let initial = compare_evidence(
            &pristine_original,
            &pristine,
            &output.join("pristine-differences"),
        )?;
        report["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"pristine_baseline","status":"complete","comparison":initial}));
        if initial["same"] == true {
            report["status"] = json!("complete");
            report["category"] = json!("identical");
            report["reason"] = json!(
                "Selected pixels are equal; metadata-only row-height evidence is retained in the primary result"
            );
            return Ok(());
        }
        let replay = self.replay_diagnosis(profile, profile, &selected, &output.join("replay"))?;
        let repeat = compare_evidence(&pristine, &replay, &output.join("replay-differences"))?;
        report["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"pristine_replay","status":"complete","comparison":repeat}));
        if repeat["same"] != true {
            report["status"] = json!("unstable");
            report["category"] = json!("unstable");
            report["reason"] = json!("Primary variant rendering did not reproduce before repair");
            return Ok(());
        }

        let mut passes = Vec::new();
        for pass in 1..=2 {
            for client in ["original", profile] {
                self.diagnosis_command(client, json!({"operation":"recalculate"}))?;
            }
            let directory = output.join(format!("recalculation-{pass}"));
            let left =
                self.replay_diagnosis(profile, "original", &selected, &directory.join("original"))?;
            let right =
                self.replay_diagnosis(profile, profile, &selected, &directory.join("variant"))?;
            passes.push((left, right));
        }
        let original_stability = compare_evidence(
            &passes[0].0,
            &passes[1].0,
            &output.join("original-stability"),
        )?;
        let variant_stability = compare_evidence(
            &passes[0].1,
            &passes[1].1,
            &output.join("variant-stability"),
        )?;
        report["steps"].as_array_mut().unwrap().push(json!({"kind":"recalculation_stability","status":"complete",
            "original":original_stability,"variant":variant_stability,
            "meaning":"Two equal observations do not prove absence of volatile or external inputs"}));
        if original_stability["same"] != true || variant_stability["same"] != true {
            report["status"] = json!("unstable");
            report["category"] = json!("unstable");
            report["reason"] = json!("Consecutive recalculations produced different renderings");
            return Ok(());
        }
        let (reference, mut current) = passes.pop().unwrap();
        report["reference_state"] = json!("paired post-recalculation snapshots");
        let baseline = compare_evidence(
            &reference,
            &current,
            &output.join("post-recalculation-differences"),
        )?;
        report["steps"].as_array_mut().unwrap().push(
            json!({"kind":"post_recalculation_baseline","status":"complete","comparison":baseline}),
        );
        if baseline["same"] == true {
            report["status"] = json!("complete");
            report["category"] = json!("recalculation");
            report["resolved_by"] = json!("recalculation");
            return Ok(());
        }

        let mut applied = Vec::<String>::new();
        let heights = height_repairs(&reference.snapshot, &current.snapshot);
        if !heights.is_empty() {
            let step = report["steps"].as_array().unwrap().len();
            report["steps"].as_array_mut().unwrap().push(json!({
                "kind":"row_height","status":"mutation_requested","height_runs":heights}));
            let response = self.diagnosis_command(
                profile,
                json!({"operation":"row_heights","heights":heights}),
            )?;
            report["steps"][step]["status"] = json!("applied");
            report["steps"][step]["mutation"] = response;
            applied.push("row_height".into());
            report["applied_steps"] = json!(applied);
            current = self.replay_diagnosis(
                profile,
                profile,
                &selected,
                &output.join("row-height/variant"),
            )?;
            let comparison =
                compare_evidence(&reference, &current, &output.join("row-height/differences"))?;
            report["steps"][step]["status"] = json!("complete");
            report["steps"][step]["comparison"] = comparison.clone();
            if comparison["same"] == true {
                return finish_resolution(report, &applied, "row_height");
            }
        } else {
            report["steps"]
                .as_array_mut()
                .unwrap()
                .push(json!({"kind":"row_height","status":"no_candidates"}));
        }

        let sheets = sheet_descriptors(&self.original)?;
        let source_sheets: Vec<_> = sheets
            .iter()
            .map(|(index, (_, file, hidden))| (*index, file.clone(), *hidden))
            .collect();
        let variant = self
            .variants
            .get(profile)
            .context("diagnosis variant source missing")?;
        let removed =
            removed_diagnostic_cells(&self.original, variant, &source_sheets, MAX_RESTORED_CELLS)?;
        let local: BTreeSet<_> = selected.tiles.iter().map(|tile| tile.sheet_index).collect();
        let mut restored = BTreeSet::<(usize, String)>::new();
        for kind in [
            "local_whitespace",
            "local_hidden_content",
            "cross_sheet_hidden_content",
            "cross_sheet_whitespace",
        ] {
            let candidates: Vec<_> = removed
                .iter()
                .filter(|cell| {
                    !restored.contains(&(cell.sheet_index, cell.cell_ref.clone()))
                        && match kind {
                            "local_whitespace" => {
                                local.contains(&cell.sheet_index) && cell.whitespace
                            }
                            "local_hidden_content" => {
                                local.contains(&cell.sheet_index) && cell.hidden && !cell.whitespace
                            }
                            "cross_sheet_hidden_content" => {
                                !local.contains(&cell.sheet_index)
                                    && cell.hidden
                                    && !cell.whitespace
                            }
                            _ => cell.whitespace,
                        }
                })
                .collect();
            if candidates.is_empty() {
                report["steps"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"kind":kind,"status":"no_candidates"}));
                continue;
            }
            let identities: Vec<_> = candidates.iter().map(|cell| {
                let name = &sheets[&cell.sheet_index].0;
                json!({"sheet_index":cell.sheet_index,"sheet_name":name,"address":cell.cell_ref,"has_formula":cell.has_formula})
            }).collect();
            let source = self.diagnosis_command(
                "original",
                json!({"operation":"read_cells","cells":identities}),
            )?;
            let cells = source["cells"]
                .as_array()
                .context("diagnosis source values missing")?;
            ensure!(
                cells.len() == candidates.len(),
                "diagnosis source values incomplete"
            );
            let step = report["steps"].as_array().unwrap().len();
            report["steps"].as_array_mut().unwrap().push(json!({"kind":kind,
                "status":"mutation_requested","restored_cell_count":cells.len(),"restored_cells":cells}));
            let recalculate = kind != "local_whitespace";
            let response = self.diagnosis_command(
                profile,
                json!({"operation":"restore_cells","cells":cells,"recalculate":recalculate}),
            )?;
            for cell in candidates {
                restored.insert((cell.sheet_index, cell.cell_ref.clone()));
            }
            report["steps"][step]["status"] = json!("applied");
            report["steps"][step]["mutation"] = response;
            applied.push(kind.into());
            report["applied_steps"] = json!(applied);
            current = self.replay_diagnosis(
                profile,
                profile,
                &selected,
                &output.join(kind).join("variant"),
            )?;
            let comparison =
                compare_evidence(&reference, &current, &output.join(kind).join("differences"))?;
            report["steps"][step]["status"] = json!("complete");
            report["steps"][step]["comparison"] = comparison.clone();
            if comparison["same"] == true {
                return finish_resolution(report, &applied, kind);
            }
        }
        report["status"] = json!("complete");
        report["category"] = json!("unresolved");
        report["applied_steps"] = json!(applied);
        Ok(())
    }
}

fn finish_resolution(report: &mut Value, applied: &[String], stage: &str) -> Result<()> {
    report["status"] = json!("complete");
    report["category"] = json!(resolved_category(applied));
    report["resolved_by"] = json!(stage);
    report["applied_steps"] = json!(applied);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_resolution_does_not_claim_an_exclusive_whitespace_cause() -> Result<()> {
        let mut report = json!({});
        finish_resolution(
            &mut report,
            &["row_height".into(), "local_whitespace".into()],
            "local_whitespace",
        )?;
        assert_eq!(report["category"], "combined");
        assert_eq!(report["resolved_by"], "local_whitespace");
        finish_resolution(
            &mut report,
            &["local_hidden_content".into()],
            "local_hidden_content",
        )?;
        assert_eq!(report["category"], "local_hidden_content");
        Ok(())
    }
    #[test]
    fn row_height_repair_stops_at_the_last_mismatch_row_and_includes_earlier_displacement()
    -> Result<()> {
        use crate::regression::row_heights::HeightRun;
        let tile: Tile = serde_json::from_value(json!({"sheet_index":1,"sheet_name":"Data",
            "tile_index":0,"range":"A8:B9","core_rows":[7,9],"capture_right_column":1,
            "width":20,"height":20,"sha256":"test"}))?;
        let reference = Snapshot {
            command: "before".into(),
            tiles: vec![tile],
            heights: vec![SheetHeights {
                sheet_index: 1,
                sheet_name: "Data".into(),
                runs: vec![
                    HeightRun {
                        start_row: 1,
                        end_row: 5,
                        height: 15.0,
                    },
                    HeightRun {
                        start_row: 6,
                        end_row: 10,
                        height: 20.0,
                    },
                ],
            }],
        };
        let mut variant = reference.clone();
        variant.heights[0].runs = vec![HeightRun {
            start_row: 1,
            end_row: 10,
            height: 15.0,
        }];
        let repairs = height_repairs(&reference, &variant);
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0]["start_row"], 6);
        assert_eq!(repairs[0]["end_row"], 9);
        assert_eq!(repairs[0]["height"], 20.0);
        Ok(())
    }

    /// Run serially after building the release CLI. Fixture directory contains
    /// original.xlsx, variant.xlsx and expected.txt (the expected category).
    #[test]
    #[ignore = "requires desktop Excel and EXCEL_SHRINK_DIAGNOSIS_FIXTURE; run with --test-threads=1"]
    fn experimental_diagnosis_excel_fixture() -> Result<()> {
        struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Environment {
            fn drop(&mut self) {
                for (name, value) in self.0.drain(..) {
                    if let Some(value) = value {
                        std::env::set_var(name, value);
                    } else {
                        std::env::remove_var(name);
                    }
                }
            }
        }
        let root = PathBuf::from(
            std::env::var_os("EXCEL_SHRINK_DIAGNOSIS_FIXTURE")
                .context("Set EXCEL_SHRINK_DIAGNOSIS_FIXTURE to a generated fixture directory")?,
        );
        let original = root.join("original.xlsx");
        let variant = root.join("variant.xlsx");
        let expected = fs::read_to_string(root.join("expected.txt"))?
            .trim()
            .to_owned();
        ensure!(
            original.is_file() && variant.is_file(),
            "diagnosis fixture inputs are missing"
        );
        let host = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/release/excel-shrink-cli.exe")
            .canonicalize()?;
        let names = [
            "EXCEL_SHRINK_CAPTURE_SWEEP",
            "EXCEL_SHRINK_CAPTURE_HOST",
            "EXCEL_SHRINK_HIDE_EXCEL",
            "EXCEL_SHRINK_CONTROL_ROOM_DIR",
            "EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS",
            "EXCEL_SHRINK_CAPTURE_TIMEOUT_SECONDS",
            "EXCEL_SHRINK_SYSTEM_RESERVE_MIB",
        ];
        let _environment = Environment(
            names
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect(),
        );
        std::env::set_var("EXCEL_SHRINK_CAPTURE_SWEEP", "1");
        std::env::set_var("EXCEL_SHRINK_CAPTURE_HOST", host);
        std::env::set_var("EXCEL_SHRINK_HIDE_EXCEL", "1");
        std::env::set_var("EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS", "90");
        std::env::set_var("EXCEL_SHRINK_CAPTURE_TIMEOUT_SECONDS", "120");
        std::env::set_var("EXCEL_SHRINK_SYSTEM_RESERVE_MIB", "0");
        std::env::remove_var("EXCEL_SHRINK_CONTROL_ROOM_DIR");
        let output = root.join(format!("diagnosis-run-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&output)?;
        let mut session = ResidentCapture::start(
            &original,
            &[("too-aggressive-cleaning", variant)],
            &output.join("resident"),
        )?;
        let primary = session.compare("too-aggressive-cleaning", &output.join("primary"))?;
        let report = session.diagnose_profile(
            "too-aggressive-cleaning",
            &primary,
            &output.join("diagnosis"),
        )?;
        let cleanup = session.finish();
        println!(
            "DIAGNOSIS_FIXTURE {}",
            serde_json::to_string(&json!({
            "fixture":root,"expected":expected,"actual":report["category"],"status":report["status"],
            "error":report["error"],"output":output}))?
        );
        cleanup?;
        ensure!(
            report["category"] == expected,
            "diagnosis category differs: expected {expected}, got {} ({})",
            report["category"],
            report["error"]
        );
        ensure!(
            report["status"]
                == if expected == "unstable" {
                    "unstable"
                } else {
                    "complete"
                },
            "diagnosis did not complete: {}",
            report["error"]
        );
        Ok(())
    }
}
