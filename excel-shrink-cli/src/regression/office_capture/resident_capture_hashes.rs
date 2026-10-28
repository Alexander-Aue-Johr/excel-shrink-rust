//! Compare retained PNG digests before transferring images. A single boundary
//! band may be parked on disk until its next neighbor has been classified.
use super::*;

#[derive(Default, Serialize)]
struct HashStats {
    captured_tiles: usize,
    hash_equal_positions: usize,
    png_mismatch_positions: usize,
    pixel_different_positions: usize,
    context_positions: usize,
    uploaded_images: usize,
    uploaded_bytes: u64,
    boundary_spill_images: usize,
    boundary_spill_bytes: u64,
    captured_png_bytes: u64,
    bytes_not_uploaded: u64,
    /// Sum of client durations; concurrent workers make this non-additive wall time.
    worker_hash_ms: f64,
}

#[derive(Default)]
struct BatchStats {
    paused: Duration,
    images: usize,
    bytes: u64,
    captured_png_bytes: u64,
    worker_hash_ms: f64,
}

/// The current capture contract consists of full-width row bands. Adjacency is
/// expressed in worksheet rows, not array positions or rendered pixel heights:
/// omitted ranges and zero-height hidden rows must not invent neighbors.
fn neighbors(tiles: &[Tile], position: usize) -> impl Iterator<Item = usize> + '_ {
    let current = &tiles[position];
    [position.checked_sub(1), position.checked_add(1)]
        .into_iter()
        .flatten()
        .filter(move |other| {
            tiles.get(*other).is_some_and(|candidate| {
                candidate.sheet_index == current.sheet_index
                    && (candidate.core_rows[1] == current.core_rows[0]
                        || current.core_rows[1] == candidate.core_rows[0])
            })
        })
}

fn equal_offers(offers: &BTreeMap<String, Vec<Tile>>, offset: usize) -> bool {
    let mut peers = offers.values().map(|tiles| &tiles[offset]);
    let Some(first) = peers.next() else {
        return false;
    };
    peers.all(|tile| {
        tile.sha256 == first.sha256 && tile.width == first.width && tile.height == first.height
    })
}

fn validate_offer(expected: &Tile, offered: &Tile) -> Result<()> {
    ensure!(
        offered.sheet_index == expected.sheet_index
            && offered.sheet_name == expected.sheet_name
            && offered.tile_index == expected.tile_index
            && offered.range == expected.range
            && offered.core_rows == expected.core_rows
            && offered.capture_right_column == expected.capture_right_column
            && offered.width == expected.width
            && offered.height == expected.height
            && offered.x == expected.x
            && offered.y == expected.y
            && offered.capture_pixels == expected.capture_pixels
            && offered.sha256.len() == 64
            && offered.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "retained PNG digest differs from the negotiated capture geometry"
    );
    Ok(())
}

fn decode_retained_images(
    clients: &BTreeSet<String>,
    snapshots: &mut BTreeMap<String, Snapshot>,
    paths: &BTreeMap<(String, String, usize, usize), PathBuf>,
    position: usize,
) -> Result<BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>> {
    let mut images = BTreeMap::new();
    for client in clients {
        let snapshot = snapshots
            .get_mut(client)
            .context("missing image snapshot")?;
        let tile = &mut snapshot.tiles[position];
        let path = paths
            .get(&(
                client.clone(),
                snapshot.command.clone(),
                tile.sheet_index,
                tile.tile_index,
            ))
            .context("missing retained PNG for exact comparison")?
            .clone();
        let pixels = image::open(&path)?.to_rgba8();
        ensure!(
            pixels.dimensions() == (tile.width, tile.height),
            "retained image dimensions changed"
        );
        // Every peer at this position changes digest domain together. PNG
        // encodings may differ even when their decoded pixels are identical.
        tile.sha256 = artifact_cache::sha256(pixels.as_raw());
        images.insert(client.clone(), (tile.clone(), path, pixels));
    }
    Ok(images)
}

impl ResidentCapture {
    pub(super) fn capture_hash_first(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        ensure!(
            clients.contains("original"),
            "hash-first requires an original client"
        );
        let reference = self.snapshot("original")?;
        let height_sheets = self.hash_height_sheets(clients, &reference)?;
        let live = self
            .live
            .as_mut()
            .context("missing hash-first live state")?;
        live.plan(&reference)?;
        live.state["transport"] = json!("PNG SHA-256 first; exact host pixels for unequal digests");
        live.state["capture_pipeline"] = json!({
            "mode":"bounded-hash-first", "transport":"hash-first",
            "window_batches":1,
            "max_pending_tiles_per_client":MAX_PENDING_TILES,
            "max_pending_pixels_per_client":MAX_PENDING_PIXELS,
            "max_pending_png_bytes_per_client":MAX_PENDING_PNG_BYTES,
            "context_policy":"touching capture bands; temporary boundary PNGs on disk",
        });
        live.save()?;

        let mut stats = HashStats::default();
        let mut contexts = BTreeMap::<usize, BTreeSet<usize>>::new();
        let mut different = BTreeSet::new();
        let mut boundary = None;
        let mut start = 0;
        while start < reference.tiles.len() {
            let requested = self.live.as_ref().unwrap().batch_tiles();
            let end = batch_end(&reference, &self.snapshots, clients, start, requested);
            let mut batch = BatchStats::default();
            let mut clock = OpeningPause::new(
                self.cleanup_barrier
                    .as_ref()
                    .and_then(|barrier| barrier.pause_path.as_deref()),
            );
            self.live
                .as_mut()
                .unwrap()
                .active(&reference.tiles[start..end])?;
            let offers = self.capture_digest_batch(clients, start, end, &mut batch)?;
            stats.captured_tiles += (end - start) * clients.len();

            let mut required = BTreeSet::new();
            for position in start..end {
                if equal_offers(&offers, position - start) {
                    stats.hash_equal_positions += 1;
                } else {
                    stats.png_mismatch_positions += 1;
                    required.insert(position);
                }
                if height_sheets.contains(&reference.tiles[position].sheet_index) {
                    required.insert(position);
                }
            }
            self.upload_hash_tiles(clients, &offers, start, &required, &mut batch)?;
            let mut processed = BTreeSet::new();
            for &position in &required {
                if self.compare_hash_images(clients, &reference, position, &height_sheets)? {
                    different.insert(position);
                    for neighbor in neighbors(&reference.tiles, position) {
                        contexts.entry(neighbor).or_default().insert(position);
                    }
                }
                processed.insert(position);
            }

            let needed_context: BTreeSet<_> = contexts
                .keys()
                .copied()
                .filter(|position| (start..end).contains(position))
                .collect();
            self.upload_hash_tiles(clients, &offers, start, &needed_context, &mut batch)?;
            let mut available_context = needed_context;
            if let Some(previous) = boundary.filter(|previous| contexts.contains_key(previous)) {
                available_context.insert(previous);
            }
            for position in available_context {
                if !processed.contains(&position) {
                    // Context candidates have already been certified equal by
                    // their PNG digest. Decode the original retained bytes;
                    // never ask Excel to recapture a historical neighbor.
                    self.compare_hash_images(clients, &reference, position, &height_sheets)?;
                    processed.insert(position);
                }
                self.publish_hash_context(clients, &reference, position, &contexts[&position])?;
            }
            for position in start..end {
                if !processed.contains(&position) {
                    self.publish_hash_equal(&reference.tiles[position], position)?;
                }
            }

            let next_boundary = (end < reference.tiles.len()
                && neighbors(&reference.tiles, end - 1).any(|neighbor| neighbor == end))
            .then_some(end - 1);
            if let Some(position) = next_boundary {
                let before = (batch.images, batch.bytes);
                self.upload_hash_tiles(
                    clients,
                    &offers,
                    start,
                    &BTreeSet::from([position]),
                    &mut batch,
                )?;
                stats.boundary_spill_images += batch.images - before.0;
                stats.boundary_spill_bytes += batch.bytes - before.1;
            }
            self.discard_hash_batch(clients, &offers, &mut batch)?;

            for position in boundary.into_iter().chain(start..end) {
                let keep = different.contains(&position)
                    || contexts.contains_key(&position)
                    || height_sheets.contains(&reference.tiles[position].sheet_index)
                    || next_boundary == Some(position);
                if !keep {
                    self.retire_hash_images(clients, position)?;
                }
            }
            boundary = next_boundary;
            stats.pixel_different_positions = different.len();
            stats.context_positions = contexts.keys().filter(|position| **position < end).count();
            stats.uploaded_images += batch.images;
            stats.uploaded_bytes += batch.bytes;
            stats.captured_png_bytes += batch.captured_png_bytes;
            stats.bytes_not_uploaded = stats
                .captured_png_bytes
                .saturating_sub(stats.uploaded_bytes);
            stats.worker_hash_ms += batch.worker_hash_ms;
            let pixels: u64 = clients
                .iter()
                .flat_map(|client| &self.snapshots[client].tiles[start..end])
                .map(|tile| {
                    tile.capture_pixels
                        .max(tile.width as u64 * tile.height as u64)
                })
                .sum();
            let core_pixels: u64 = clients
                .iter()
                .flat_map(|client| &self.snapshots[client].tiles[start..end])
                .map(|tile| tile.width as u64 * tile.height as u64)
                .sum();
            let live = self.live.as_mut().unwrap();
            live.state["capture_pipeline"]["hash_first"] = serde_json::to_value(&stats)?;
            live.state["pipeline_timings"] = serde_json::to_value(&self.timings)?;
            live.state["event_transport"] = self.events.snapshot();
            live.record_batch(
                pixels,
                core_pixels,
                batch.images,
                batch.bytes,
                clock.active_elapsed().saturating_sub(batch.paused),
                requested,
                end - start,
                false,
            )?;
            start = end;
        }
        let live = self.live.as_mut().unwrap();
        live.state["primary_complete"] = json!(true);
        live.state["status"] = json!("comparing");
        live.save()
    }

    fn hash_height_sheets(
        &self,
        clients: &BTreeSet<String>,
        reference: &Snapshot,
    ) -> Result<BTreeSet<usize>> {
        let baseline: BTreeMap<_, _> = reference
            .heights
            .iter()
            .map(|sheet| Ok((sheet.sheet_index, serde_json::to_value(sheet)?)))
            .collect::<Result<_>>()?;
        let mut changed = BTreeSet::new();
        for client in clients {
            for sheet in &self.snapshots[client].heights {
                if baseline.get(&sheet.sheet_index) != Some(&serde_json::to_value(sheet)?) {
                    changed.insert(sheet.sheet_index);
                }
            }
        }
        Ok(changed)
    }

    /// Publish every peer job before waiting. Results remain durable on disk,
    /// and normal pause/opening/restart handling stays shared with live capture.
    fn hash_commands(
        &mut self,
        commands: BTreeMap<String, Value>,
        batch: &mut BatchStats,
    ) -> Result<BTreeMap<String, Value>> {
        let mut pending = Vec::new();
        for (client, body) in commands {
            let command = self.issue(&client, body)?;
            pending.push(PendingCapture {
                client,
                command,
                capture: String::new(),
                tiles: vec![],
                received: BTreeSet::new(),
                clock: None,
                user_pause: Duration::ZERO,
                sampled: Instant::now(),
            });
        }
        let mut results = BTreeMap::new();
        let mut paused = Duration::ZERO;
        while !pending.is_empty() {
            let CaptureEvent::Response(index) = self.next_live_event(&mut pending)? else {
                bail!("unexpected image event while awaiting retained-image command");
            };
            let job = pending.remove(index);
            let result = self.wait(&job.client, &job.command)?;
            paused = paused.max(job.user_pause);
            results.insert(job.client, result);
        }
        batch.paused += paused;
        Ok(results)
    }

    fn capture_digest_batch(
        &mut self,
        clients: &BTreeSet<String>,
        start: usize,
        end: usize,
        batch: &mut BatchStats,
    ) -> Result<BTreeMap<String, Vec<Tile>>> {
        ensure!(
            start < end && end - start <= MAX_PENDING_TILES,
            "invalid retained capture batch size"
        );
        for client in clients {
            let pixels: u64 = self.snapshots[client].tiles[start..end]
                .iter()
                .map(|tile| {
                    tile.capture_pixels
                        .max(tile.width as u64 * tile.height as u64)
                })
                .sum();
            ensure!(
                pixels <= MAX_PENDING_PIXELS,
                "retained capture batch exceeds pixel budget"
            );
        }
        let commands = clients
            .iter()
            .map(|client| {
                let snapshot = &self.snapshots[client];
                (
                    client.clone(),
                    json!({"kind":"capture_hashes", "capture_id":snapshot.command,
                "tiles":snapshot.tiles[start..end], "capture_batches":[end-start]}),
                )
            })
            .collect();
        let results = self.hash_commands(commands, batch)?;
        let mut offers = BTreeMap::new();
        for (client, result) in results {
            batch.worker_hash_ms += result["timings"]["hash_ms"]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(0.0);
            let items = result["tiles"]
                .as_array()
                .context("missing retained digest tiles")?;
            let mut retained_bytes = 0;
            for item in items {
                let bytes = item["png_bytes"]
                    .as_u64()
                    .context("missing retained PNG byte count")?;
                ensure!(
                    bytes > 0 && bytes <= MAX_PENDING_PNG_BYTES,
                    "invalid retained PNG byte count"
                );
                batch.captured_png_bytes += bytes;
                retained_bytes += bytes;
            }
            ensure!(
                retained_bytes <= MAX_PENDING_PNG_BYTES,
                "retained batch exceeds encoded-image budget"
            );
            let tiles: Vec<Tile> = serde_json::from_value(result["tiles"].clone())?;
            ensure!(
                tiles.len() == end - start,
                "incomplete retained digest batch"
            );
            for (offset, tile) in tiles.iter().enumerate() {
                validate_offer(&self.snapshots[&client].tiles[start + offset], tile)?;
                self.snapshots.get_mut(&client).unwrap().tiles[start + offset] = tile.clone();
            }
            offers.insert(client, tiles);
        }
        Ok(offers)
    }

    fn upload_hash_tiles(
        &mut self,
        clients: &BTreeSet<String>,
        offers: &BTreeMap<String, Vec<Tile>>,
        start: usize,
        positions: &BTreeSet<usize>,
        batch: &mut BatchStats,
    ) -> Result<()> {
        let mut expected = BTreeMap::<String, Vec<Tile>>::new();
        let mut commands = BTreeMap::new();
        for client in clients {
            let capture = &self.snapshots[client].command;
            let missing: Vec<_> = positions
                .iter()
                .filter(|position| {
                    let tile = &offers[client][**position - start];
                    !self.images.contains_key(&(
                        client.clone(),
                        capture.clone(),
                        tile.sheet_index,
                        tile.tile_index,
                    ))
                })
                .map(|position| offers[client][*position - start].clone())
                .collect();
            if missing.is_empty() {
                continue;
            }
            commands.insert(
                client.clone(),
                json!({"kind":"upload_retained", "capture_id":capture,"tiles":missing}),
            );
            expected.insert(client.clone(), missing);
        }
        for (client, result) in self.hash_commands(commands, batch)? {
            let items = result["images"]
                .as_array()
                .context("missing retained-image receipts")?;
            ensure!(
                items.len() == expected[&client].len(),
                "incomplete retained-image uploads"
            );
            for (item, expected) in items.iter().zip(&expected[&client]) {
                let tile: Tile = serde_json::from_value(item.clone())?;
                validate_offer(expected, &tile)?;
                ensure!(
                    tile.sha256 == expected.sha256,
                    "retained image no longer matches its offered PNG digest"
                );
                let path = fs::canonicalize(
                    item["image_path"]
                        .as_str()
                        .context("missing retained PNG path")?,
                )?;
                ensure!(
                    path.starts_with(self.directory.join("images")),
                    "retained image outside session"
                );
                let bytes = fs::metadata(&path)?.len();
                ensure!(
                    bytes <= MAX_PENDING_PNG_BYTES,
                    "retained PNG exceeds image byte budget"
                );
                ensure!(
                    artifact_cache::hash_file(&path)? == tile.sha256,
                    "retained PNG digest mismatch"
                );
                let reader = image::ImageReader::open(&path)?.with_guessed_format()?;
                ensure!(
                    reader.format() == Some(image::ImageFormat::Png),
                    "retained image is not PNG"
                );
                ensure!(
                    reader.into_dimensions()? == (tile.width, tile.height),
                    "retained PNG dimensions mismatch"
                );
                self.images.insert(
                    (
                        client.clone(),
                        self.snapshots[&client].command.clone(),
                        tile.sheet_index,
                        tile.tile_index,
                    ),
                    path,
                );
                batch.images += 1;
                batch.bytes += bytes;
                self.timings.images_received += 1;
                for (name, total) in [
                    ("upload_receive_ms", &mut self.timings.upload_receive_ms),
                    (
                        "upload_validate_publish_ms",
                        &mut self.timings.upload_validate_publish_ms,
                    ),
                ] {
                    if let Some(value) = item["timings"][name]
                        .as_f64()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                    {
                        *total += value;
                    }
                }
            }
        }
        Ok(())
    }

    fn compare_hash_images(
        &mut self,
        clients: &BTreeSet<String>,
        reference: &Snapshot,
        position: usize,
        height_sheets: &BTreeSet<usize>,
    ) -> Result<bool> {
        let started = Instant::now();
        let images = decode_retained_images(clients, &mut self.snapshots, &self.images, position)?;
        self.timings.decode_ms += started.elapsed().as_secs_f64() * 1000.0;
        let different = images.values().any(|(_, _, pixels)| {
            !visually_equal_with_blank_margins(&images["original"].2, pixels)
        });
        let started = Instant::now();
        let target = &reference.tiles[position];
        let live = self.live.as_mut().unwrap();
        live.retain_tile(
            target,
            &self.aliases,
            &images,
            height_sheets.contains(&target.sheet_index),
        )?;
        if let Some(tile) = live.tile_mut(target) {
            tile["comparison_method"] = json!("rgba");
        }
        live.save()?;
        self.timings.compare_ms += started.elapsed().as_secs_f64() * 1000.0;
        Ok(different)
    }

    fn publish_hash_equal(&mut self, reference: &Tile, position: usize) -> Result<()> {
        let images: BTreeMap<_, _> = self
            .aliases
            .iter()
            .map(|(logical, physical)| {
                let tile = &self.snapshots[physical].tiles[position];
                (
                    logical,
                    json!({
                        "width":tile.width,"height":tile.height,"x":tile.x,"y":tile.y,
                        "pixel_class":format!("{}x{}:png:{}", tile.width,tile.height,tile.sha256),
                        "changed_pixels":0,"url":null,"preview_url":null,"full_url":null,
                        "mask_url":null,"retained_full_resolution":false,
                    }),
                )
            })
            .collect();
        let live = self.live.as_mut().unwrap();
        if let Some(tile) = live.tile_mut(reference) {
            tile["status"] = json!("equal");
            tile["equal_across_all"] = json!(true);
            tile["comparison_method"] = json!("png-sha256");
            tile["images"] = json!(images);
            tile["updated_at"] = json!(now_ms());
        }
        live.save()
    }

    fn publish_hash_context(
        &mut self,
        clients: &BTreeSet<String>,
        reference: &Snapshot,
        position: usize,
        differences: &BTreeSet<usize>,
    ) -> Result<()> {
        let target = &reference.tiles[position];
        let live = self.live.as_mut().unwrap();
        let mut urls = BTreeMap::new();
        for client in clients {
            let snapshot = &self.snapshots[client];
            let tile = &snapshot.tiles[position];
            let path = &self.images[&(
                client.clone(),
                snapshot.command.clone(),
                tile.sheet_index,
                tile.tile_index,
            )];
            let name = format!(
                "s{}-t{}-{client}.png",
                target.sheet_index, target.tile_index
            );
            fs::copy(path, live.images_directory().join(&name))?;
            urls.insert(client, live.image_url(&name));
        }
        if let Some(tile) = live.tile_mut(target) {
            tile["context_for"] = json!(differences
                .iter()
                .map(|position| {
                    let difference = &reference.tiles[*position];
                    format!("s{}-t{}", difference.sheet_index, difference.tile_index)
                })
                .collect::<Vec<_>>());
            for (logical, physical) in &self.aliases {
                if let Some(url) = urls.get(physical) {
                    let image = &mut tile["images"][logical];
                    image["full_url"] = json!(url);
                    image["retained_full_resolution"] = json!(true);
                }
            }
        }
        live.save()
    }

    fn discard_hash_batch(
        &mut self,
        clients: &BTreeSet<String>,
        offers: &BTreeMap<String, Vec<Tile>>,
        batch: &mut BatchStats,
    ) -> Result<()> {
        let commands = clients.iter().map(|client| (client.clone(), json!({
            "kind":"discard_retained", "capture_id":self.snapshots[client].command,
            "tiles":offers[client].iter().map(|tile| json!({"sheet_index":tile.sheet_index,"tile_index":tile.tile_index})).collect::<Vec<_>>()
        }))).collect();
        self.hash_commands(commands, batch)?;
        Ok(())
    }

    fn retire_hash_images(&mut self, clients: &BTreeSet<String>, position: usize) -> Result<()> {
        for client in clients {
            let snapshot = &self.snapshots[client];
            let tile = &snapshot.tiles[position];
            if let Some(path) = self.images.remove(&(
                client.clone(),
                snapshot.command.clone(),
                tile.sheet_index,
                tile.tile_index,
            )) {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(sheet: usize, rows: [u32; 2]) -> Tile {
        Tile {
            sheet_index: sheet,
            sheet_name: format!("Sheet {sheet}"),
            tile_index: rows[0] as usize,
            range: format!("A{}:B{}", rows[0] + 1, rows[1]),
            core_rows: rows,
            capture_right_column: 1,
            width: 20,
            height: 20,
            x: 0,
            y: rows[0] * 20,
            capture_pixels: 400,
            pixel_budget: 0,
            capture_budget_fallback: None,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn neighbors_touch_in_worksheet_coordinates_without_crossing_omitted_rows() {
        let tiles = vec![
            tile(0, [0, 10]),
            tile(0, [10, 20]),
            tile(0, [20, 30]),
            tile(0, [100, 110]),
            tile(1, [0, 10]),
        ];
        assert_eq!(neighbors(&tiles, 0).collect::<Vec<_>>(), vec![1]);
        assert_eq!(neighbors(&tiles, 1).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(neighbors(&tiles, 2).collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            neighbors(&tiles, 3).collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
        assert_eq!(
            neighbors(&tiles, 4).collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn equal_offers_require_matching_dimensions_as_well_as_digest() {
        let mut peers = BTreeMap::from([
            ("original".into(), vec![tile(0, [0, 10])]),
            ("default".into(), vec![tile(0, [0, 10])]),
        ]);
        assert!(equal_offers(&peers, 0));
        peers.get_mut("default").unwrap()[0].width += 1;
        assert!(!equal_offers(&peers, 0));
        peers.get_mut("default").unwrap()[0].width -= 1;
        peers.get_mut("default").unwrap()[0].sha256 = "b".repeat(64);
        assert!(!equal_offers(&peers, 0));
    }

    #[test]
    fn digest_offer_cannot_replace_the_planned_geometry() {
        let expected = tile(0, [0, 10]);
        let mut changed = expected.clone();
        changed.sha256 = "b".repeat(64);
        assert!(validate_offer(&expected, &changed).is_ok());
        changed.y += 1;
        assert!(validate_offer(&expected, &changed).is_err());
        changed.y = expected.y;
        changed.sha256 = "not a checksum".into();
        assert!(validate_offer(&expected, &changed).is_err());
    }

    #[test]
    fn unequal_png_encodings_need_exact_pixels_before_marking_a_difference() -> Result<()> {
        use image::codecs::png::{CompressionType, FilterType, PngEncoder};
        use image::ImageEncoder;

        for changed_pixel in [false, true] {
            let root = artifact_cache::unique_path(&std::env::temp_dir(), "hash-first-pixels")?;
            fs::create_dir_all(&root)?;
            let original = image::RgbaImage::from_pixel(20, 20, image::Rgba([22, 50, 91, 255]));
            let mut variant = original.clone();
            if changed_pixel {
                variant.put_pixel(7, 11, image::Rgba([220, 50, 91, 255]));
            }
            let original_path = root.join("original.png");
            let variant_path = root.join("default.png");
            original.save(&original_path)?;
            let mut bytes = Vec::new();
            PngEncoder::new_with_quality(&mut bytes, CompressionType::Best, FilterType::NoFilter)
                .write_image(variant.as_raw(), 20, 20, image::ExtendedColorType::Rgba8)?;
            fs::write(&variant_path, bytes)?;
            assert_ne!(fs::read(&original_path)?, fs::read(&variant_path)?);

            let mut snapshots = BTreeMap::<String, Snapshot>::new();
            let mut paths = BTreeMap::<(String, String, usize, usize), PathBuf>::new();
            for (client, path) in [("original", original_path), ("default", variant_path)] {
                let mut metadata = tile(0, [0, 10]);
                metadata.sha256 = artifact_cache::hash_file(&path)?;
                paths.insert((client.into(), "plan".into(), 0, 0), path);
                snapshots.insert(
                    client.into(),
                    Snapshot {
                        command: "plan".into(),
                        heights: vec![],
                        tiles: vec![metadata],
                    },
                );
            }
            let offers = snapshots
                .iter()
                .map(|(client, snapshot)| (client.clone(), snapshot.tiles.clone()))
                .collect();
            assert!(!equal_offers(&offers, 0));
            let clients = BTreeSet::from(["original".into(), "default".into()]);
            let images = decode_retained_images(&clients, &mut snapshots, &paths, 0)?;
            assert_eq!(images["original"].2 != images["default"].2, changed_pixel);
            assert_eq!(
                snapshots["original"].tiles[0].sha256 != snapshots["default"].tiles[0].sha256,
                changed_pixel
            );

            let aliases = clients
                .iter()
                .map(|client| (client.clone(), client.clone()))
                .collect();
            let mut live = LiveCapture {
                root: root.clone(),
                directory: root.join("dashboard"),
                id: "fixture".into(),
                state: json!({"sheets":[]}),
                controls: RefCell::new(json!({"paused":false})),
            };
            fs::create_dir_all(live.images_directory())?;
            live.plan(&snapshots["original"])?;
            let keep =
                live.retain_tile(&snapshots["original"].tiles[0], &aliases, &images, false)?;
            assert_eq!(keep, changed_pixel);
            assert_eq!(
                live.state["sheets"][0]["tiles"][0]["status"],
                if changed_pixel { "different" } else { "equal" }
            );
            assert_eq!(
                live.images_directory()
                    .join("s0-t0-default-mask.png")
                    .is_file(),
                changed_pixel
            );
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }
}
