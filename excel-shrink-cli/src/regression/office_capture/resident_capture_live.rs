//! Image-first capture: one common tile is in flight per workbook group.
//! Full images cross the local endpoint once; equality is decided on the host.
use super::*;
use std::cell::RefCell;

pub(super) struct LiveCapture {
    root: PathBuf,
    directory: PathBuf,
    id: String,
    state: Value,
    controls: RefCell<Value>,
}

impl LiveCapture {
    pub(super) fn from_environment(
        session: &Path,
        original: &Path,
        aliases: &BTreeMap<String, String>,
    ) -> Result<Option<Self>> {
        let Some(root) = std::env::var_os("EXCEL_SHRINK_CONTROL_ROOM_DIR") else {
            return Ok(None);
        };
        let root = PathBuf::from(root);
        let id = std::env::var("EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID").unwrap_or_else(|_| {
            artifact_cache::sha256(session.to_string_lossy().as_bytes())[..20].to_owned()
        });
        ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid control room group id"
        );
        let directory = root.join("groups").join(&id);
        fs::create_dir_all(directory.join("images"))?;
        let profiles: Vec<_> = [
            "original",
            "default",
            "maximum-cleaning",
            "too-aggressive-cleaning",
        ]
        .iter()
        .filter(|id| aliases.contains_key(**id))
        .map(|id| json!({"id":id,"label":label(id),"capture_client":aliases[*id]}))
        .collect();
        let live = Self {
            root,
            directory,
            id,
            state: json!({
                "version":1,
                "source_path":original,"session_directory":session,"status":"opening",
                "profiles":profiles,"aliases":aliases,"sheets":[],"primary_complete":false,
                "transport":"PNG uploads; exact decoded pixel comparison on host",
            }),
            controls: RefCell::new(json!({"paused":true})),
        };
        live.save()?;
        Ok(Some(live))
    }

    fn controls(&self) -> Value {
        if let Some(value) = fs::read(self.root.join("control.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|value| value.is_object() && value["paused"].is_boolean())
        {
            *self.controls.borrow_mut() = value;
        }
        self.controls.borrow().clone()
    }

    pub(super) fn paused(&self) -> bool {
        self.controls()["paused"].as_bool().unwrap_or(true)
    }
    pub(super) fn job_timeout(&self, fallback: Duration) -> Duration {
        self.controls()["job_timeout_seconds"]
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or(fallback)
    }
    fn preview_width(&self) -> u32 {
        self.controls()["equal_preview_width"]
            .as_u64()
            .filter(|width| [0, 128, 256, 512, 1024].contains(width))
            .unwrap_or(512) as u32
    }
    fn save(&self) -> Result<()> {
        publish(&self.directory.join("state.json"), &self.state)
    }
    pub(super) fn planning(&mut self) -> Result<()> {
        self.state["status"] = json!("planning");
        self.save()
    }
    pub(super) fn complete(&mut self) -> Result<()> {
        self.state["status"] = json!("complete");
        self.state["primary_complete"] = json!(true);
        self.save()
    }
    fn plan(&mut self, snapshot: &Snapshot) -> Result<()> {
        if !self.state["sheets"].as_array().unwrap().is_empty() {
            return Ok(());
        }
        let mut sheets: BTreeMap<usize, Value> = BTreeMap::new();
        let mut offsets = BTreeMap::<usize, u32>::new();
        for tile in &snapshot.tiles {
            let y = offsets.entry(tile.sheet_index).or_default();
            let sheet = sheets.entry(tile.sheet_index).or_insert_with(|| json!({"id":tile.sheet_index.to_string(),"index":tile.sheet_index,"name":tile.sheet_name,"tiles":[]}));
            sheet["tiles"].as_array_mut().unwrap().push(json!({
                "id":format!("s{}-t{}",tile.sheet_index,tile.tile_index),"tile_index":tile.tile_index,
                "address":tile.range,"range":tile.range,"row":tile.core_rows[0],"col":0,
                "core_rows":tile.core_rows,"capture_right_column":tile.capture_right_column,
                "x":0,"y":*y,"width":tile.width,"height":tile.height,"status":"pending","images":{}
            }));
            *y += tile.height;
        }
        self.state["sheets"] = json!(sheets.into_values().collect::<Vec<_>>());
        self.state["status"] = json!("capturing");
        self.save()
    }
    fn active(&mut self, tile: &Tile) -> Result<()> {
        self.state["active_sheet"] = json!(tile.sheet_index.to_string());
        self.state["active_tile"] = json!(format!("s{}-t{}", tile.sheet_index, tile.tile_index));
        if let Some(target) = self.tile_mut(tile) {
            target["status"] = json!("capturing");
        }
        self.save()
    }
    fn received(
        &mut self,
        reference: &Tile,
        physical: &str,
        tile: &Tile,
        pixels: &image::RgbaImage,
        aliases: &BTreeMap<String, String>,
        image_y: u32,
    ) -> Result<()> {
        let name = format!(
            "s{}-t{}-{physical}-preview.png",
            reference.sheet_index, reference.tile_index
        );
        let preview = preview(pixels, 1024);
        save_png(&preview, &self.directory.join("images").join(&name))?;
        let timestamp = now_ms();
        let url = format!("/data/groups/{}/images/{name}", self.id);
        let metadata = json!({"url":url,"preview_url":url,"pixel_class":format!("{}x{}:{}",tile.width,tile.height,tile.sha256),
            "mask_url":null,"full_url":null,"changed_pixels":null,"width":tile.width,"height":tile.height,
            "x":0,"y":image_y,"status":"received","updated_at":timestamp});
        if let Some(target) = self.tile_mut(reference) {
            for (logical, client) in aliases {
                if client == physical {
                    target["images"][logical] = metadata.clone();
                }
            }
            target["status"] = json!("receiving");
            target["updated_at"] = json!(timestamp);
        }
        self.save()
    }
    fn tile_mut(&mut self, tile: &Tile) -> Option<&mut Value> {
        self.state["sheets"]
            .as_array_mut()?
            .iter_mut()
            .find(|sheet| sheet["index"] == tile.sheet_index)?["tiles"]
            .as_array_mut()?
            .iter_mut()
            .find(|entry| entry["address"] == tile.range)
    }

    // Scientific evidence keeps its exact PNG. Dashboard previews are separate
    // assets and never participate in comparison or the whitespace certificate.
    fn retain_tile(
        &mut self,
        reference: &Tile,
        aliases: &BTreeMap<String, String>,
        images: &BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>,
        keep_for_heights: bool,
        positions: &BTreeMap<String, u32>,
    ) -> Result<bool> {
        ensure!(
            aliases.values().all(|client| images.contains_key(client)),
            "cannot classify a tile before every required peer has uploaded"
        );
        let Some((_, _, original)) = images.get("original") else {
            return Ok(true);
        };
        let equal = images.values().all(|(_, _, image)| image == original);
        // Row-height reports may still need the exact source PNG even when
        // the displayed pixels match. That scientific evidence is independent
        // of the user's resolution setting for equal dashboard tiles.
        let keep_evidence = !equal || keep_for_heights;
        let full = !equal;
        let preview_width = self.preview_width();
        let timestamp = now_ms();
        let prefix = format!("s{}-t{}", reference.sheet_index, reference.tile_index);
        let mut urls = BTreeMap::<String, Value>::new();
        let mut union = image::RgbaImage::new(original.width(), original.height());
        for (physical, (tile, path, pixels)) in images {
            let basename = format!("{prefix}-{physical}");
            let full_path = self
                .directory
                .join("images")
                .join(format!("{basename}.png"));
            if full {
                fs::copy(path, &full_path)?;
            }
            let preview_path = self
                .directory
                .join("images")
                .join(format!("{basename}-preview.png"));
            let max_width = if full { 1024 } else { preview_width };
            let scaled = preview(pixels, max_width);
            save_png(&scaled, &preview_path)?;
            let (mask, changed) = difference_mask(original, pixels);
            let mask_name = format!("{basename}-mask.png");
            if changed > 0 {
                save_png(&mask, &self.directory.join("images").join(&mask_name))?;
                for (x, y, pixel) in union.enumerate_pixels_mut() {
                    if x >= pixels.width()
                        || y >= pixels.height()
                        || original.get_pixel(x, y) != pixels.get_pixel(x, y)
                    {
                        *pixel = image::Rgba([255, 30, 55, 210]);
                    }
                }
            }
            let url = |name: &str| format!("/data/groups/{}/images/{name}", self.id);
            urls.insert(physical.clone(), json!({
                "url":url(&format!("{basename}-preview.png")),"preview_url":url(&format!("{basename}-preview.png")),
                "full_url":if full {Some(url(&format!("{basename}.png")))} else if scaled.dimensions()==pixels.dimensions() {Some(url(&format!("{basename}-preview.png")))} else {None},
                "mask_url":if changed > 0 {Some(url(&mask_name))} else {None},
                "pixel_class":format!("{}x{}:{}",pixels.width(),pixels.height(),tile.sha256),
                "changed_pixels":changed,"width":pixels.width(),"height":pixels.height(),"x":0,"y":positions.get(physical).copied().unwrap_or(0),
                "retained_full_resolution":full || scaled.dimensions()==pixels.dimensions(),"updated_at":timestamp,
            }));
        }
        if union.pixels().any(|pixel| pixel[3] != 0) {
            let name = format!("{prefix}-original-mask.png");
            save_png(&union, &self.directory.join("images").join(&name))?;
            urls.get_mut("original").unwrap()["mask_url"] =
                json!(format!("/data/groups/{}/images/{name}", self.id));
        }
        let logical: BTreeMap<_, _> = aliases
            .iter()
            .filter_map(|(profile, physical)| {
                urls.get(physical)
                    .map(|value| (profile.clone(), value.clone()))
            })
            .collect();
        if let Some(tile) = self.tile_mut(reference) {
            tile["status"] = json!(if equal { "equal" } else { "different" });
            tile["equal_across_all"] = json!(equal);
            tile["images"] = json!(logical);
            tile["updated_at"] = json!(timestamp);
        }
        self.save()?;
        Ok(keep_evidence)
    }
}

fn label(id: &str) -> &str {
    match id {
        "original" => "Original",
        "default" => "Default",
        "maximum-cleaning" => "Maximum cleaning",
        "too-aggressive-cleaning" => "Too aggressive",
        _ => id,
    }
}
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(super) fn client_ready_for_jobs(session: &Path, client: &str) -> bool {
    fs::read(session.join("metrics.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|metrics| {
            metrics["primary_clients_ready"] == true
                && metrics["clients"][client]["status"] == "ready"
        })
}

fn preview(pixels: &image::RgbaImage, max_width: u32) -> image::RgbaImage {
    if max_width == 0 || pixels.width() <= max_width {
        pixels.clone()
    } else {
        image::imageops::resize(
            pixels,
            max_width,
            ((pixels.height() as u64 * max_width as u64) / pixels.width() as u64).max(1) as u32,
            image::imageops::FilterType::Triangle,
        )
    }
}
fn save_png(pixels: &image::RgbaImage, path: &Path) -> Result<()> {
    let pending = path.with_extension("pending.png");
    pixels.save(&pending)?;
    fs::rename(pending, path)?;
    Ok(())
}

fn difference_mask(
    original: &image::RgbaImage,
    variant: &image::RgbaImage,
) -> (image::RgbaImage, u64) {
    let mut mask = image::RgbaImage::new(variant.width(), variant.height());
    let mut changed = 0;
    for (x, y, pixel) in mask.enumerate_pixels_mut() {
        if x >= original.width()
            || y >= original.height()
            || variant.get_pixel(x, y) != original.get_pixel(x, y)
        {
            *pixel = image::Rgba([255, 30, 55, 210]);
            changed += 1;
        }
    }
    // Pixels removed by a smaller image are differences too, even though their
    // mask only fits on the original panel.
    changed += (original.width() as u64 * original.height() as u64)
        - original.width().min(variant.width()) as u64
            * original.height().min(variant.height()) as u64;
    (mask, changed)
}

impl ResidentCapture {
    pub(super) fn capture_synchronized(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        let control_only = !clients.contains("original");
        let reference_client = if control_only {
            clients.first().context("no live clients")?.as_str()
        } else {
            "original"
        };
        let reference = self.snapshot(reference_client)?;
        if !control_only {
            self.live.as_mut().unwrap().plan(&reference)?;
        }
        // Row-height evidence may need an image even when pixels are identical.
        let height_sheets: BTreeSet<_> = if control_only {
            BTreeSet::new()
        } else {
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
            changed
        };
        let mut offsets = BTreeMap::<(String, usize), u32>::new();
        for position in 0..reference.tiles.len() {
            let target = &reference.tiles[position];
            if !control_only {
                self.live.as_mut().unwrap().active(target)?;
            }
            // Issue every peer's one-tile job before waiting for any response.
            let mut commands = Vec::new();
            for client in clients {
                let snapshot = &self.snapshots[client];
                let capture = snapshot.command.clone();
                let tile = snapshot.tiles[position].clone();
                let id = self.issue(
                    client,
                    json!({"kind":"capture_images","capture_id":capture,"tiles":[tile]}),
                )?;
                commands.push(PendingCapture {
                    client: client.clone(),
                    capture,
                    command: id,
                    tile,
                    clock: None,
                    user_pause: Duration::ZERO,
                    sampled: Instant::now(),
                });
            }
            let mut received = BTreeMap::new();
            let mut positions = BTreeMap::new();
            while !commands.is_empty() {
                let ready = self.next_live_response(&mut commands)?;
                let PendingCapture {
                    client,
                    capture,
                    command,
                    mut tile,
                    ..
                } = commands.remove(ready);
                let result = self.wait(&client, &command)?;
                let receipts = result["images"]
                    .as_array()
                    .context("missing image-first receipts")?;
                ensure!(
                    receipts.len() == 1,
                    "image-first job must return exactly one tile"
                );
                let receipt = &receipts[0];
                let metadata: Tile = serde_json::from_value(receipt.clone())?;
                ensure!(
                    metadata.sheet_index == tile.sheet_index
                        && metadata.tile_index == tile.tile_index
                        && metadata.range == tile.range
                        && metadata.width == tile.width
                        && metadata.height == tile.height
                        && metadata.core_rows == tile.core_rows
                        && metadata.capture_right_column == tile.capture_right_column,
                    "image-first receipt differs from negotiated tile"
                );
                let path =
                    fs::canonicalize(receipt["image_path"].as_str().context("missing PNG path")?)?;
                ensure!(
                    path.starts_with(self.directory.join("images")),
                    "image-first PNG outside session"
                );
                ensure!(
                    fs::metadata(&path)?.len() <= 160 * 1024 * 1024,
                    "image-first PNG too large"
                );
                ensure!(
                    artifact_cache::hash_file(&path)? == metadata.sha256,
                    "image-first receipt digest mismatch"
                );
                let pixels = image::open(&path)?.to_rgba8();
                ensure!(
                    pixels.dimensions() == (tile.width, tile.height),
                    "image-first decoded dimensions differ"
                );
                tile.sha256 = artifact_cache::sha256(pixels.as_raw());
                self.snapshots.get_mut(&client).unwrap().tiles[position] = tile.clone();
                self.images.insert(
                    (client.clone(), capture, tile.sheet_index, tile.tile_index),
                    path.clone(),
                );
                let image_y = offsets
                    .entry((client.clone(), tile.sheet_index))
                    .or_default();
                positions.insert(client.clone(), *image_y);
                if !control_only {
                    self.live.as_mut().unwrap().received(
                        target,
                        &client,
                        &tile,
                        &pixels,
                        &self.aliases,
                        *image_y,
                    )?;
                }
                *image_y += tile.height;
                received.insert(client, (tile, path, pixels));
            }
            if !control_only {
                let keep = self.live.as_mut().unwrap().retain_tile(
                    target,
                    &self.aliases,
                    &received,
                    height_sheets.contains(&target.sheet_index),
                    &positions,
                )?;
                if !keep {
                    for (client, (tile, path, _)) in received {
                        self.images.remove(&(
                            client.clone(),
                            self.snapshots[&client].command.clone(),
                            tile.sheet_index,
                            tile.tile_index,
                        ));
                        fs::remove_file(path)?;
                    }
                }
            }
        }
        if !control_only {
            let live = self.live.as_mut().unwrap();
            live.state["primary_complete"] = json!(true);
            live.state["status"] = json!("comparing");
            live.save()?;
        }
        Ok(())
    }

    fn next_live_response(&mut self, pending: &mut [PendingCapture]) -> Result<usize> {
        loop {
            if let Some(ready) = ready_response(&self.directory, pending) {
                return Ok(ready);
            }
            let failure = self.directory.join("launcher-error.json");
            ensure!(
                !failure.is_file(),
                "resident Excel launcher failed: {}",
                fs::read_to_string(failure).unwrap_or_default()
            );
            if let Some(status) = self.child.try_wait()? {
                bail!("resident Excel launcher exited ({status}) while waiting for tile uploads")
            }
            let live = self.live.as_ref().unwrap();
            let paused = live.paused();
            let timeout = live.job_timeout(self.timeout);
            for job in pending.iter_mut() {
                let now = Instant::now();
                if job.clock.is_none()
                    && (client_ready_for_jobs(&self.directory, &job.client)
                        || self
                            .directory
                            .join("dispatched")
                            .join(&job.client)
                            .join(format!("{}.json", job.command))
                            .is_file())
                {
                    job.clock = Some(OpeningPause::new(
                        self.cleanup_barrier
                            .as_ref()
                            .and_then(|barrier| barrier.pause_path.as_deref()),
                    ));
                    job.sampled = now;
                    job.user_pause = Duration::ZERO;
                }
                if paused {
                    job.user_pause += now.duration_since(job.sampled);
                }
                job.sampled = now;
                if let Some(clock) = &mut job.clock {
                    ensure!(
                        clock.active_elapsed().saturating_sub(job.user_pause) < timeout,
                        "resident tile deadline exceeded: {}/{}",
                        job.client,
                        job.command
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

struct PendingCapture {
    client: String,
    capture: String,
    command: String,
    tile: Tile,
    clock: Option<OpeningPause>,
    user_pause: Duration,
    sampled: Instant,
}

fn ready_response(directory: &Path, pending: &[PendingCapture]) -> Option<usize> {
    pending
        .iter()
        .enumerate()
        .filter_map(|(index, job)| {
            fs::metadata(
                directory
                    .join("responses")
                    .join(&job.client)
                    .join(format!("{}.json", job.command)),
            )
            .ok()
            .map(|metadata| (index, metadata.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .min_by_key(|(_, time)| *time)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_masks_include_changed_alpha_and_removed_pixels() {
        let original = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let mut variant = original.clone();
        variant.put_pixel(1, 1, image::Rgba([1, 2, 3, 254]));
        let (mask, count) = difference_mask(&original, &variant);
        assert_eq!(count, 1);
        assert_eq!(mask.get_pixel(0, 0)[3], 0);
        assert_ne!(mask.get_pixel(1, 1)[3], 0);
        let smaller = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        assert_eq!(difference_mask(&original, &smaller).1, 2);
    }

    #[test]
    fn invalid_control_reads_preserve_pause_and_positive_job_timeouts() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-controls")?;
        fs::create_dir_all(&root)?;
        let live = LiveCapture {
            root: root.clone(),
            directory: root.clone(),
            id: "fixture".into(),
            state: json!({}),
            controls: RefCell::new(json!({"paused":true})),
        };
        assert!(
            live.paused(),
            "missing initial controls must not start work"
        );
        fs::write(
            root.join("control.json"),
            br#"{"paused":true,"job_timeout_seconds":1}"#,
        )?;
        assert_eq!(
            live.job_timeout(Duration::from_secs(100)),
            Duration::from_secs(1)
        );
        fs::write(root.join("control.json"), b"{broken")?;
        assert!(live.paused());
        fs::write(
            root.join("control.json"),
            br#"{"paused":false,"job_timeout_seconds":2}"#,
        )?;
        assert!(!live.paused());
        assert_eq!(
            live.job_timeout(Duration::from_secs(100)),
            Duration::from_secs(2)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn queued_jobs_get_a_deadline_after_all_peers_and_the_target_are_ready() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "queued-live-job")?;
        fs::create_dir(&root)?;
        assert!(!client_ready_for_jobs(&root, "original"));
        fs::write(
            root.join("metrics.json"),
            br#"{"primary_clients_ready":false,"clients":{"original":{"status":"ready"}}}"#,
        )?;
        assert!(!client_ready_for_jobs(&root, "original"));
        fs::write(root.join("metrics.json"),br#"{"primary_clients_ready":true,"clients":{"original":{"status":"ready"},"control-default":{"status":"opening"}}}"#)?;
        assert!(client_ready_for_jobs(&root, "original"));
        assert!(!client_ready_for_jobs(&root, "control-default"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn retained_tiles_group_exact_pixels_and_keep_differences_at_full_resolution() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-retention")?;
        let directory = root.join("groups").join("fixture");
        fs::create_dir_all(directory.join("images"))?;
        fs::write(
            root.join("control.json"),
            br#"{"paused":false,"equal_preview_width":128}"#,
        )?;
        let mut live = LiveCapture {
            root: root.clone(),
            directory: directory.clone(),
            id: "fixture".into(),
            state: json!({"sheets":[]}),
            controls: RefCell::new(json!({"paused":true})),
        };
        let tile = Tile {
            sheet_index: 1,
            sheet_name: "Sheet1".into(),
            tile_index: 0,
            range: "A1:B2".into(),
            core_rows: [0, 2],
            capture_right_column: 1,
            width: 256,
            height: 2,
            sha256: String::new(),
        };
        let snapshot = Snapshot {
            tiles: vec![tile.clone()],
            heights: vec![],
            command: "plan".into(),
        };
        live.plan(&snapshot)?;
        let original = image::RgbaImage::from_pixel(256, 2, image::Rgba([255, 255, 255, 255]));
        let mut different = original.clone();
        different.put_pixel(100, 1, image::Rgba([2, 3, 4, 255]));
        let aliases = BTreeMap::from([
            ("original".into(), "original".into()),
            ("default".into(), "original".into()),
            ("maximum-cleaning".into(), "maximum-cleaning".into()),
            ("too-aggressive-cleaning".into(), "maximum-cleaning".into()),
        ]);
        let mut images = BTreeMap::new();
        for (id, pixels) in [
            ("original", original.clone()),
            ("maximum-cleaning", different),
        ] {
            let path = root.join(format!("{id}.png"));
            pixels.save(&path)?;
            let mut metadata = tile.clone();
            metadata.sha256 = artifact_cache::sha256(pixels.as_raw());
            images.insert(id.into(), (metadata, path, pixels));
        }
        assert!(live
            .retain_tile(
                &tile,
                &aliases,
                &BTreeMap::from([("original".into(), images["original"].clone())]),
                false,
                &BTreeMap::new()
            )
            .is_err());
        assert!(live.retain_tile(&tile, &aliases, &images, false, &BTreeMap::new())?);
        let actual = &live.state["sheets"][0]["tiles"][0];
        assert_eq!(actual["status"], "different");
        assert_eq!(actual["images"]["maximum-cleaning"]["changed_pixels"], 1);
        assert_eq!(
            actual["images"]["maximum-cleaning"]["pixel_class"],
            actual["images"]["too-aggressive-cleaning"]["pixel_class"]
        );
        assert_eq!(
            actual["images"]["original"]["pixel_class"],
            actual["images"]["default"]["pixel_class"]
        );
        assert_eq!(
            image::image_dimensions(directory.join("images/s1-t0-maximum-cleaning.png"))?,
            (256, 2)
        );
        assert!(actual["images"]["original"]["mask_url"].is_string());
        // A separate equal tile retains the configured preview, never a PNG
        // whose reduced resolution could be mistaken for comparison evidence.
        let mut equal_tile = tile.clone();
        equal_tile.tile_index = 1;
        equal_tile.range = "A3:B4".into();
        live.state["sheets"] = json!([]);
        live.plan(&Snapshot {
            tiles: vec![equal_tile.clone()],
            ..snapshot
        })?;
        let equal_images = BTreeMap::from([("original".into(), images["original"].clone())]);
        assert!(!live.retain_tile(
            &equal_tile,
            &BTreeMap::from([("original".into(), "original".into())]),
            &equal_images,
            false,
            &BTreeMap::new()
        )?);
        assert_eq!(
            image::image_dimensions(directory.join("images/s1-t1-original-preview.png"))?,
            (128, 1)
        );
        assert!(!directory.join("images/s1-t1-original.png").exists());
        assert!(
            root.join("original.png").is_file(),
            "fixture evidence must not be overwritten"
        );
        let pending = vec!["original", "maximum-cleaning"]
            .into_iter()
            .map(|client| PendingCapture {
                client: client.into(),
                capture: "plan".into(),
                command: "tile".into(),
                tile: tile.clone(),
                clock: None,
                user_pause: Duration::ZERO,
                sampled: Instant::now(),
            })
            .collect::<Vec<_>>();
        fs::create_dir_all(root.join("responses/maximum-cleaning"))?;
        fs::write(root.join("responses/maximum-cleaning/tile.json"), b"{}")?;
        assert_eq!(
            ready_response(&root, &pending),
            Some(1),
            "a later peer must not wait behind an unfinished original"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn equal_tiles_obey_preview_resolution_while_row_height_evidence_stays_full() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "equal-height-preview")?;
        let directory = root.join("groups/fixture");
        fs::create_dir_all(directory.join("images"))?;
        fs::write(
            root.join("control.json"),
            br#"{"paused":false,"equal_preview_width":256}"#,
        )?;
        let mut live = LiveCapture {
            root: root.clone(),
            directory: directory.clone(),
            id: "fixture".into(),
            state: json!({"sheets":[]}),
            controls: RefCell::new(json!({"paused":true})),
        };
        let tile = Tile {
            sheet_index: 1,
            sheet_name: "Sheet1".into(),
            tile_index: 0,
            range: "A1:B2".into(),
            core_rows: [0, 2],
            capture_right_column: 1,
            width: 1024,
            height: 8,
            sha256: "test-pixels".into(),
        };
        live.plan(&Snapshot {
            tiles: vec![tile.clone()],
            heights: vec![],
            command: "plan".into(),
        })?;
        let pixels = image::RgbaImage::from_pixel(1024, 8, image::Rgba([255, 255, 255, 255]));
        let source = root.join("scientific-evidence.png");
        pixels.save(&source)?;
        let images = BTreeMap::from([("original".into(), (tile.clone(), source.clone(), pixels))]);
        let aliases = BTreeMap::from([("original".into(), "original".into())]);
        assert!(
            live.retain_tile(&tile, &aliases, &images, true, &BTreeMap::new())?,
            "caller must retain row-height evidence"
        );
        let rendered = &live.state["sheets"][0]["tiles"][0];
        assert_eq!(rendered["status"], "equal");
        assert!(rendered["images"]["original"]["full_url"].is_null());
        assert_eq!(
            rendered["images"]["original"]["retained_full_resolution"],
            false
        );
        assert_eq!(
            image::image_dimensions(directory.join("images/s1-t0-original-preview.png"))?,
            (256, 2)
        );
        assert!(!directory.join("images/s1-t0-original.png").exists());
        assert_eq!(image::image_dimensions(&source)?, (1024, 8));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
