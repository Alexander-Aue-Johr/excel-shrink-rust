//! Experimental whole-workbook hash sweeps and targeted late profile captures.
use super::*;

fn context_distance(live: Option<&LiveCapture>) -> Result<usize> {
    let distance = if let Some(value) =
        live.and_then(|live| live.controls()["capture_context_tiles"].as_u64())
    {
        usize::try_from(value).context("capture context distance exceeds platform limit")?
    } else {
        std::env::var("EXCEL_SHRINK_CAPTURE_CONTEXT_TILES")
            .unwrap_or_else(|_| "0".into())
            .parse::<usize>()
            .context("capture context distance must be an integer")?
    };
    ensure!(
        distance <= 16,
        "capture context distance must be between 0 and 16"
    );
    Ok(distance)
}
fn with_context(tiles: &[Tile], selected: &BTreeSet<usize>, distance: usize) -> BTreeSet<usize> {
    let mut result = selected.clone();
    let mut frontier = selected.clone();
    for _ in 0..distance {
        let mut next = BTreeSet::new();
        for position in frontier {
            let tile = &tiles[position];
            for other in [position.checked_sub(1), position.checked_add(1)]
                .into_iter()
                .flatten()
            {
                if let Some(peer) = tiles.get(other) {
                    if tile.sheet_index == peer.sheet_index
                        && (tile.core_rows[1] == peer.core_rows[0]
                            || peer.core_rows[1] == tile.core_rows[0])
                        && result.insert(other)
                    {
                        next.insert(other);
                    }
                }
            }
        }
        frontier = next;
    }
    result
}

fn mismatch_selection(original: &Snapshot, primary: &Value, distance: usize) -> Result<Snapshot> {
    let sheets = primary["sheets"]
        .as_array()
        .context("missing primary visual sheets")?;
    let mut ranges = BTreeSet::new();
    let mut height_sheets = BTreeSet::new();
    for sheet in sheets {
        let index = sheet["index"]
            .as_u64()
            .context("primary sheet index missing")? as usize;
        if sheet["row_heights"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
        {
            height_sheets.insert(index);
        }
        for difference in sheet["image_differences"].as_array().into_iter().flatten() {
            let range = difference["range"]
                .as_str()
                .context("primary difference range missing")?;
            ranges.insert((index, range.to_owned()));
        }
    }
    ensure!(
        ranges.iter().all(|(sheet, range)| original
            .tiles
            .iter()
            .any(|tile| tile.sheet_index == *sheet && tile.range == *range)),
        "primary mismatch range is outside the original sweep"
    );
    let selected: BTreeSet<_> = original
        .tiles
        .iter()
        .enumerate()
        .filter(|(_, tile)| {
            ranges.contains(&(tile.sheet_index, tile.range.clone()))
                || height_sheets.contains(&tile.sheet_index)
        })
        .map(|(position, _)| position)
        .collect();
    ensure!(
        !selected.is_empty(),
        "primary comparison has no target regions"
    );
    let selected = with_context(&original.tiles, &selected, distance);
    let tiles: Vec<_> = selected
        .into_iter()
        .map(|position| original.tiles[position].clone())
        .collect();
    let wanted: BTreeSet<_> = tiles.iter().map(|tile| tile.sheet_index).collect();
    Ok(Snapshot {
        command: original.command.clone(),
        tiles,
        heights: original
            .heights
            .iter()
            .filter(|sheet| wanted.contains(&sheet.sheet_index))
            .cloned()
            .collect(),
    })
}

fn selected_sweep_configs(snapshot: &Snapshot, configs: &[Value]) -> Result<Vec<Value>> {
    let mut configs = configs.to_vec();
    for config in &mut configs {
        let index = config["index"].as_u64().context("missing sheet index")? as usize;
        let tiles: Vec<_> = snapshot.tiles.iter()
            .filter(|tile| tile.sheet_index == index).collect();
        let first = tiles.first().context("selected sheet has no captures")?;
        config["tile_rows"] = json!(tiles.iter().map(|tile| tile.core_rows).collect::<Vec<_>>());
        config["capture_end_column"] = json!(first.capture_right_column);
    }
    Ok(configs)
}

impl ResidentCapture {
    pub(in crate::regression) fn capture_selected_sweep(
        &mut self,
        clients: &BTreeSet<String>,
    ) -> Result<()> {
        let distance = context_distance(self.live.as_ref())?;
        let reference = self.snapshot("original")?;
        let sampled = self.capture_coverage.values()
            .any(|coverage| coverage["omitted_tiles"].as_u64().unwrap_or(0) > 0);
        if let Some(live) = &mut self.live {
            // Selection already published the full plan with explicit omitted bands.
            if self.capture_coverage.is_empty() {
                live.plan(&reference)?;
                live.plan_peers(&self.snapshots, &self.aliases)?;
            }
            live.state["status"] = json!("hashing");
            live.state["capture_settings"]["full_sweep"] = json!(!sampled);
            live.state["capture_settings"]["sampling_applies"] = json!(sampled);
            live.state["capture_pipeline"] = json!({"mode":"autonomous-hash-sweep", "context_distance":distance,
                "retained_images_per_client":0,"transport":"hash-sweep","hash_domain":"finalized PNG SHA-256","sweep_complete":false});
            live.save()?;
        }
        // Queue every peer before waiting: a fast client does not wait for other
        // Excel instances between tiles, and its result retains metadata only.
        for client in clients {
            let configs = selected_sweep_configs(&self.snapshots[client], &self.configs[client])?;
            self.configs.insert(client.clone(), configs.clone());
            let command = self.issue(
                client,
                json!({"kind":"capture","sheets":configs,
                "tile_megapixels":self.live.as_ref().map_or(32, |live|live.tile_megapixels())}),
            )?;
            self.pending.insert(client.clone(), command);
        }
        for client in clients {
            self.snapshot(client)?;
        }
        let reference = self.snapshot("original")?;
        for client in clients {
            ensure!(
                same_ranges(&reference, &self.snapshot(client)?),
                "full sweep changed negotiated tile ranges"
            );
        }
        let mut changed_height_sheets = BTreeSet::new();
        let original_heights: BTreeMap<_, _> = reference
            .heights
            .iter()
            .map(|sheet| Ok((sheet.sheet_index, serde_json::to_value(sheet)?)))
            .collect::<Result<_>>()?;
        for client in clients {
            for sheet in &self.snapshots[client].heights {
                if original_heights.get(&sheet.sheet_index) != Some(&serde_json::to_value(sheet)?) {
                    changed_height_sheets.insert(sheet.sheet_index);
                }
            }
        }
        let mismatches: BTreeSet<_> = reference
            .tiles
            .iter()
            .enumerate()
            .filter(|(position, tile)| {
                clients.iter().any(|client| {
                    let other = &self.snapshots[client].tiles[*position];
                    tile.sha256 != other.sha256
                        || tile.width != other.width
                        || tile.height != other.height
                }) || changed_height_sheets.contains(&tile.sheet_index)
            })
            .map(|(position, _)| position)
            .collect();
        let required = with_context(&reference.tiles, &mismatches, distance);
        if let Some(live) = &mut self.live {
            live.state["status"] = json!("capturing_evidence");
            live.state["capture_pipeline"]["sweep_complete"] = json!(true);
            live.state["capture_pipeline"]["captured_positions"] = json!(reference.tiles.len());
            live.state["capture_pipeline"]["hash_mismatch_positions"] = json!(mismatches.len());
            live.state["capture_pipeline"]["requested_evidence_positions"] = json!(required.len());
            live.state["capture_pipeline"]["replay_verified"] = json!(false);
            live.save()?;
        }
        // The analysis caller retrieves mismatch evidence before recalculation.
        // Do not request a second copy here or schedule a later sheet replay.
        if self.screening_mode {
            return Ok(());
        }
        let mut commands = Vec::new();
        for client in clients {
            let snapshot = self.snapshot(client)?;
            let tiles: Vec<_> = required
                .iter()
                .map(|position| snapshot.tiles[*position].clone())
                .collect();
            let command = self.request_images(client, &snapshot, &tiles)?;
            commands.push((client.clone(), snapshot, tiles, command));
        }
        for (client, snapshot, tiles, command) in commands {
            if let Err(error) = self.receive_images(&client, &snapshot, command, &tiles) {
                if let Some(live) = &mut self.live {
                    let message = format!("{error:#}");
                    live.state["status"] = json!(if message.contains("CAPTURE_CHANGED") {
                        "unstable_capture"
                    } else {
                        "failed"
                    });
                    live.state["capture_pipeline"]["replay_error"] = json!(message);
                    live.save()?;
                }
                return Err(error);
            }
        }
        if self.live.is_some() {
            for (position, tile) in reference.tiles.iter().enumerate() {
                if required.contains(&position) {
                    let mut images = BTreeMap::new();
                    for client in clients {
                        let snapshot = &self.snapshots[client];
                        let peer = &snapshot.tiles[position];
                        let path = self.images[&(
                            client.clone(),
                            snapshot.command.clone(),
                            peer.sheet_index,
                            peer.tile_index,
                        )]
                            .clone();
                        let decode = Instant::now();
                        let pixels = image::open(&path)?.to_rgba8();
                        ensure!(
                            pixels.dimensions() == (peer.width, peer.height),
                            "sweep image geometry changed"
                        );
                        self.timings.decode_ms += decode.elapsed().as_secs_f64() * 1000.0;
                        images.insert(client.clone(), (peer.clone(), path, pixels));
                    }
                    let clock = Instant::now();
                    let live = self.live.as_mut().unwrap();
                    live.retain_tile(
                        tile,
                        &self.aliases,
                        &images,
                        changed_height_sheets.contains(&tile.sheet_index),
                    )?;
                    live.tile_mut(tile).unwrap()["comparison_method"] =
                        json!("exact replay pixels after PNG hash sweep");
                    self.timings.compare_ms += clock.elapsed().as_secs_f64() * 1000.0;
                } else {
                    self.publish_hash_equal(tile, position)?;
                }
            }
            let live = self.live.as_mut().unwrap();
            live.state["capture_pipeline"]["sweep_complete"] = json!(true);
            live.state["capture_pipeline"]["captured_positions"] = json!(reference.tiles.len());
            live.state["capture_pipeline"]["hash_mismatch_positions"] = json!(mismatches.len());
            live.state["capture_pipeline"]["requested_evidence_positions"] = json!(required.len());
            live.state["capture_pipeline"]["replay_verified"] = json!(true);
            live.save()?;
        }
        Ok(())
    }

    pub(in crate::regression) fn add_profile(&mut self, profile: &str, path: &Path) -> Result<()> {
        ensure!(
            self.sweep_mode,
            "late profiles require the experimental sweep mode"
        );
        ensure!(
            !self.aliases.contains_key(profile),
            "profile is already open: {profile}"
        );
        let plan = plan_pair(&self.original, path)?;
        let configs: Vec<_> = plan
            .sheets
            .iter()
            .map(|sheet| sheet.shrunk_config.clone())
            .collect();
        self.aliases.insert(profile.into(), profile.into());
        self.configs.insert(profile.into(), configs.clone());
        self.plans.insert(profile.into(), plan);
        self.variants.insert(profile.into(), path.to_owned());
        publish(
            &self
                .directory
                .join("clients")
                .join(format!("{profile}.json")),
            &json!({
                "client_id":profile,"source_path":windows_path(path)?,"workbook":format!("{profile}.xlsx"),"sheets":configs
            }),
        )?;
        if let Some(live) = &mut self.live {
            live.state["aliases"] = json!(self.aliases);
            live.state["profiles"]
                .as_array_mut()
                .context("missing live profiles")?
                .push(
                    json!({"id":profile,"label":label(profile),"capture_client":profile,
                    "comparison_scope":"aggressive mismatch regions only"}),
                );
            live.save()?;
        }
        Ok(())
    }

    pub(in crate::regression) fn compare_selected(
        &mut self,
        profile: &str,
        output: &Path,
        primary: &Value,
    ) -> Result<Value> {
        ensure!(
            self.sweep_mode,
            "targeted profiles require experimental sweep mode"
        );
        let original = self
            .compared_originals
            .get("too-aggressive-cleaning")
            .or_else(|| self.compared_originals.values().next())
            .context("primary sweep has not been compared")?;
        let left = mismatch_selection(original, primary, context_distance(self.live.as_ref())?)?;
        let wanted: BTreeSet<_> = left.tiles.iter().map(|tile| tile.sheet_index).collect();
        let mut plan = self
            .plans
            .get(profile)
            .context("late profile has not been added")?
            .clone();
        plan.sheets
            .retain(|sheet| wanted.contains(&sheet.sheet_index));
        for sheet in &mut plan.sheets {
            let tiles: Vec<_> = left
                .tiles
                .iter()
                .filter(|tile| tile.sheet_index == sheet.sheet_index)
                .collect();
            let bottom = tiles.iter().map(|tile| tile.core_rows[1]).max().unwrap() - 1;
            let end = tiles[0].capture_right_column;
            sheet.bottom_row = sheet.bottom_row.max(bottom);
            sheet.row_height_bottom_row = sheet.row_height_bottom_row.max(bottom);
            sheet.capture_right_column = end;
            sheet.shrunk_config["bottom_row"] = json!(sheet.bottom_row);
            sheet.shrunk_config["capture_right_column"] = json!(end);
            sheet.shrunk_config["capture_end_column"] = json!(end);
            sheet.shrunk_config["tile_rows"] =
                json!(tiles.iter().map(|tile| tile.core_rows).collect::<Vec<_>>());
        }
        let configs: Vec<_> = plan
            .sheets
            .iter()
            .map(|sheet| sheet.shrunk_config.clone())
            .collect();
        self.recapture(profile, configs)?;
        let mut right = self.snapshot(profile)?;
        ensure!(
            same_ranges(&left, &right),
            "late profile changed selected worksheet ranges"
        );
        self.capture_target_images(profile, &mut right)?;
        self.snapshots
            .insert(self.client(profile)?.to_owned(), right.clone());
        self.publish_targeted_images(&left)?;
        let captured = self.compare_snapshots(profile, &plan, &left, &right, output)?;
        let options = visual_validation::VisualCheckOptions {
            profile: profile.into(),
            combined_report: Some(output.join("report.html")),
            ..Default::default()
        };
        visual_validation::write_captured_visuals(
            &plan,
            &self.original,
            output,
            &options,
            captured,
        )?;
        let mut result: Value =
            serde_json::from_slice(&fs::read(output.join("visual-result.json"))?)?;
        result["transport"] = serde_json::from_slice(&fs::read(output.join("transport.json"))?)?;
        result["transport"]["method"] = json!("targeted PNG capture; exact host pixels");
        result["transport"]["image_first"] = json!(true);
        result["transport"]["client_checksum_tiles"] = json!(0);
        result["capture_method"] =
            json!("selected aggressive mismatch regions; direct PNG capture");
        result["checks_complete"] = json!(false);
        result["targeted_checks_complete"] = json!(true);
        result["comparison_scope"] = json!("aggressive mismatch regions only");
        result["checked_ranges"] = json!(
            left.tiles
                .iter()
                .map(|tile| json!({"sheet_index":tile.sheet_index,"range":tile.range}))
                .collect::<Vec<_>>()
        );
        self.compared_originals
            .entry(profile.into())
            .or_insert(left);
        Ok(result)
    }

    pub(in crate::regression::office_capture::resident_capture) fn publish_targeted_images(&mut self, reference: &Snapshot) -> Result<()> {
        if self.live.is_none() {
            return Ok(());
        }
        for (position, tile) in reference.tiles.iter().enumerate() {
            let mut images = BTreeMap::new();
            for physical in self.aliases.values().collect::<BTreeSet<_>>() {
                let Some(snapshot) = self.snapshots.get(physical) else {
                    continue;
                };
                let Some(peer) = snapshot
                    .tiles
                    .iter()
                    .find(|peer| peer.sheet_index == tile.sheet_index && peer.range == tile.range)
                else {
                    continue;
                };
                let Some(path) = self.images.get(&(
                    physical.clone(),
                    snapshot.command.clone(),
                    peer.sheet_index,
                    peer.tile_index,
                )) else {
                    continue;
                };
                let pixels = image::open(path)?.to_rgba8();
                ensure!(
                    pixels.dimensions() == (peer.width, peer.height),
                    "targeted evidence dimensions changed"
                );
                images.insert(physical.clone(), (peer.clone(), path.clone(), pixels));
            }
            if images.is_empty() {
                self.publish_hash_equal(tile, position)?;
                continue;
            }
            let aliases: BTreeMap<_, _> = self
                .aliases
                .iter()
                .filter(|(_, physical)| images.contains_key(*physical))
                .map(|(name, physical)| (name.clone(), physical.clone()))
                .collect();
            let live = self.live.as_mut().unwrap();
            let previous = live
                .tile_mut(tile)
                .map(|entry| entry["images"].clone())
                .unwrap_or(Value::Null);
            live.retain_tile(tile, &aliases, &images, false)?;
            if let Some(target) = live.tile_mut(tile) {
                if let Some(previous) = previous.as_object() {
                    let current = target["images"]
                        .as_object_mut()
                        .context("live image map missing")?;
                    for (name, image) in previous {
                        current.entry(name.clone()).or_insert_with(|| image.clone());
                    }
                }
                target["comparison_scope"] =
                    json!("selected analysis ranges; omitted ranges are not checked");
            }
            live.save()?;
        }
        Ok(())
    }
    pub(in crate::regression::office_capture::resident_capture) fn capture_target_images(
        &mut self,
        profile: &str,
        snapshot: &mut Snapshot,
    ) -> Result<()> {
        let client = self.client(profile)?.to_owned();
        let clients = BTreeSet::from([client.clone()]);
        let mut snapshots = BTreeMap::from([(client.clone(), snapshot.clone())]);
        let mut start = 0;
        while start < snapshot.tiles.len() {
            let end = batch_end(snapshot, &snapshots, &clients, start, MAX_PENDING_TILES);
            let request = snapshot.tiles[start..end].to_vec();
            let command = self.issue(
                profile,
                json!({"kind":"capture_images","capture_id":snapshot.command,"tiles":request}),
            )?;
            let result = self.wait(profile, &command)?;
            let images = result["images"]
                .as_array()
                .context("targeted capture returned no images")?;
            ensure!(
                images.len() == end - start,
                "targeted capture returned an incomplete tile batch"
            );
            let mut seen = BTreeSet::new();
            for item in images {
                let actual: Tile = serde_json::from_value(item.clone())?;
                let target = snapshot.tiles[start..end]
                    .iter_mut()
                    .find(|tile| {
                        tile.sheet_index == actual.sheet_index
                            && tile.tile_index == actual.tile_index
                    })
                    .context("targeted capture returned an unknown tile")?;
                ensure!(
                    seen.insert((actual.sheet_index, actual.tile_index)),
                    "targeted capture returned duplicate tiles"
                );
                ensure!(
                    target.range == actual.range
                        && target.width == actual.width
                        && target.height == actual.height
                        && target.core_rows == actual.core_rows
                        && target.capture_right_column == actual.capture_right_column,
                    "targeted capture changed negotiated geometry"
                );
                target.sha256 = actual.sha256;
            }
            self.receive_image_result(profile, snapshot, &snapshot.tiles[start..end], &result)?;
            snapshots.insert(client.clone(), snapshot.clone());
            start = end;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "opens an explicitly supplied original/default fixture in Excel"]
    fn selected_analysis_retains_raw_and_recalculated_evidence_in_one_session() -> Result<()> {
        let root = PathBuf::from(std::env::var_os("EXCEL_SHRINK_ANALYSIS_FIXTURE")
            .context("set EXCEL_SHRINK_ANALYSIS_FIXTURE")?);
        let directory = artifact_cache::unique_path(&root, "analysis")?;
        let mut session = ResidentCapture::start_screening(&root.join("original.xlsx"),
            "default", &root.join("default.xlsx"), &directory)?;
        let result = session.screen_twice("default");
        let finished = session.finish();
        let result = result?;
        let metrics = finished?;
        ensure!(result["analysis_complete"] == true, "analysis did not finish");
        for key in ["raw_visual", "recalculated_visual"] {
            ensure!(result[key]["sheets"].as_array().is_some_and(|sheets| sheets.iter()
                .any(|sheet| sheet["status"] == "changed")), "fixture difference missing in {key}");
        }
        fs::write(root.join("analysis-result.json"), serde_json::to_vec_pretty(&result)?)?;
        fs::write(root.join("analysis-metrics.json"), serde_json::to_vec_pretty(&metrics)?)?;
        Ok(())
    }
    #[test]
    fn sparse_sweep_preserves_omitted_bands_on_both_passes() -> Result<()> {
        let snapshot = Snapshot { command: "plan".into(), heights: vec![], tiles: vec![
            tile(0, 1, [0, 100]), tile(9999, 1, [999900, 1000000]),
        ] };
        let configs = vec![json!({"index":1,"bottom_row":999999})];
        let first = selected_sweep_configs(&snapshot, &configs)?;
        assert_eq!(first[0]["tile_rows"], json!([[0,100],[999900,1000000]]));
        assert_eq!(selected_sweep_configs(&snapshot, &first)?, first);
        assert!(selected_sweep_configs(&snapshot, &[json!({"index":2})]).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "opens an explicitly supplied original/default fixture in Excel"]
    fn existing_tall_workbook_hash_sweep() -> Result<()> {
        let root = PathBuf::from(std::env::var_os("EXCEL_SHRINK_SWEEP_FIXTURE")
            .context("set EXCEL_SHRINK_SWEEP_FIXTURE")?);
        let directory = artifact_cache::unique_path(&root, "capture")?;
        let mut session = ResidentCapture::start_screening(&root.join("original.xlsx"),
            "default", &root.join("default.xlsx"), &directory)?;
        let result = session.screen_twice("default");
        let finished = session.finish();
        let result = result?;
        finished?;
        fs::write(root.join("screen.json"), serde_json::to_vec_pretty(&result)?)?;
        let sheets = result["sheets"].as_array().context("missing sheets")?;
        ensure!(sheets.iter().any(|sheet| sheet["capture_coverage"]["omitted_tiles"].as_u64().unwrap_or(0) > 0),
            "fixture did not exercise tall-sheet omissions");
        for sheet in sheets {
            println!("{}: {}", sheet["sheet_name"], sheet["capture_coverage"]);
        }
        Ok(())
    }
    fn tile(index: usize, sheet: usize, rows: [u32; 2]) -> Tile {
        Tile {
            sheet_index: sheet,
            sheet_name: format!("Sheet{sheet}"),
            tile_index: index,
            range: format!("A{}:B{}", rows[0] + 1, rows[1]),
            core_rows: rows,
            capture_right_column: 1,
            width: 2,
            height: 2,
            x: 0,
            y: 0,
            capture_pixels: 4,
            pixel_budget: 32_000_000,
            capture_budget_fallback: None,
            sha256: "a".repeat(64),
        }
    }
    #[test]
    fn zero_context_keeps_only_selected_tiles_and_context_never_crosses_gaps_or_sheets() {
        let tiles = vec![
            tile(0, 1, [0, 2]),
            tile(1, 1, [2, 4]),
            tile(2, 1, [8, 10]),
            tile(0, 2, [10, 12]),
        ];
        let selected = BTreeSet::from([1]);
        assert_eq!(with_context(&tiles, &selected, 0), selected);
        assert_eq!(with_context(&tiles, &selected, 3), BTreeSet::from([0, 1]));
    }
    #[test]
    fn targeted_selection_includes_visual_mismatches_and_preserves_exact_original_hashes()
    -> Result<()> {
        let original = Snapshot {
            command: "first".into(),
            heights: vec![],
            tiles: vec![tile(0, 1, [0, 2]), tile(1, 1, [2, 4])],
        };
        let primary = json!({"sheets":[{"index":1,"row_heights":[],"image_differences":[{"range":"A3:B4"}]}]});
        let selected = mismatch_selection(&original, &primary, 0)?;
        assert_eq!(selected.tiles.len(), 1);
        assert_eq!(selected.tiles[0].sha256, original.tiles[1].sha256);
        assert_eq!(selected.command, "first");
        Ok(())
    }
    #[test]
    fn selected_regions_outside_the_primary_sweep_are_rejected() {
        let original = Snapshot {
            command: "first".into(),
            heights: vec![],
            tiles: vec![tile(0, 1, [0, 2])],
        };
        let primary = json!({"sheets":[{"index":1,"row_heights":[],"image_differences":[
            {"range":"A1:B2"},{"range":"A3:B4"}
        ]}]});
        assert!(mismatch_selection(&original, &primary, 0).is_err());
    }
}
