//! Image-first capture: bounded, common tile batches per workbook group.
//! Full images cross the local endpoint once; equality is decided on the host.
use super::*;
use std::cell::RefCell;

const MAX_PENDING_TILES: usize = 16;
const MAX_PENDING_PIXELS: u64 = 64_000_000;
const MAX_PENDING_PNG_BYTES: u64 = 160 * 1024 * 1024;

#[path = "resident_image_processing.rs"]
mod processing;

#[path = "resident_capture_pipeline.rs"]
mod pipeline;

/// The coordinator keeps reading controls while the image thread owns the UI.
struct LiveControls {
    root: PathBuf,
    cached: RefCell<Value>,
}

fn read_controls(root: &Path, cached: &RefCell<Value>) -> Value {
    if let Some(value) = fs::read(root.join("control.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|value| value.is_object() && value["paused"].is_boolean())
    {
        *cached.borrow_mut() = value;
    }
    cached.borrow().clone()
}

impl LiveControls {
    fn from_live(live: &LiveCapture) -> Self {
        Self {
            root: live.root.clone(),
            cached: RefCell::new(live.controls()),
        }
    }

    fn read(&self) -> Value {
        read_controls(&self.root, &self.cached)
    }
}

/// Totals measured with each process's monotonic clock. Host upload durations
/// can overlap coordinator work; these fields are not additive wall time.
#[derive(Default, Serialize)]
pub(super) struct PipelineTimings {
    pub wait_ms: f64,
    pub decode_ms: f64,
    pub preview_ms: f64,
    pub compare_ms: f64,
    pub upload_receive_ms: f64,
    pub upload_validate_publish_ms: f64,
    pub commands_completed: u64,
    pub images_received: u64,
}

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
        let attempt = std::env::var("EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID").ok();
        if let Some(id) = &attempt {
            ensure!(
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "invalid control room attempt id"
            );
        }
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
        let mut live = Self {
            root,
            directory,
            id,
            state: json!({
                "version":1,
                "source_path":original,"session_directory":session,"status":"opening",
                "profiles":profiles,"aliases":aliases,"sheets":[],"primary_complete":false,
                "attempt_id":attempt,
                "transport":"PNG uploads; exact decoded pixel comparison on host",
            }),
            controls: RefCell::new(json!({"paused":true})),
        };
        fs::create_dir_all(live.images_directory())?;
        let controls = live.controls();
        let megapixels = std::env::var("EXCEL_SHRINK_CAPTURE_TILE_MEGAPIXELS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| controls["capture_tile_megapixels"].as_u64())
            .filter(|value| (1..=32).contains(value))
            .unwrap_or(32);
        let batch_tiles = std::env::var("EXCEL_SHRINK_CAPTURE_BATCH_TILES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| controls["capture_batch_tiles"].as_u64())
            .filter(|value| (1..=16).contains(value))
            .unwrap_or(1);
        live.state["capture_settings"] = json!({
            "tile_megapixels":megapixels,"batch_tiles":batch_tiles,
            "batch_max_megapixels":64,"layout":"full-width guarded row bands",
            "tile_size_applies":"next group","batch_size_applies":"next group",
            "tall_sheet_threshold":controls["tall_sheet_threshold"].as_u64()
                .or_else(|| std::env::var("EXCEL_SHRINK_TALL_SHEET_THRESHOLD").ok()?.parse().ok()).unwrap_or(40),
            "tall_sheet_edge_tiles":controls["tall_sheet_edge_tiles"].as_u64()
                .or_else(|| std::env::var("EXCEL_SHRINK_TALL_SHEET_EDGE_TILES").ok()?.parse().ok()).unwrap_or(2)
        });
        live.save()?;
        Ok(Some(live))
    }

    fn controls(&self) -> Value {
        read_controls(&self.root, &self.controls)
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
    pub(super) fn tile_megapixels(&self) -> u64 {
        self.state["capture_settings"]["tile_megapixels"]
            .as_u64()
            .unwrap_or(32)
    }
    pub(super) fn tall_sheet_setting(&self, key: &str) -> Option<u64> {
        self.state["capture_settings"][key].as_u64()
    }
    pub(super) fn selected_plan(
        &mut self,
        snapshot: &Snapshot,
        snapshots: &BTreeMap<String, Snapshot>,
        aliases: &BTreeMap<String, String>,
        coverage: &BTreeMap<usize, Value>,
        keep: &BTreeSet<(usize, usize)>,
    ) -> Result<()> {
        self.plan(snapshot)?;
        self.plan_peers(snapshots, aliases)?;
        for sheet in self.state["sheets"].as_array_mut().unwrap() {
            let index = sheet["index"].as_u64().unwrap() as usize;
            sheet["capture_coverage"] = coverage[&index].clone();
            for tile in sheet["tiles"].as_array_mut().unwrap() {
                let tile_index = tile["tile_index"].as_u64().unwrap() as usize;
                if !keep.contains(&(index, tile_index)) {
                    tile["status"] = json!("omitted");
                }
            }
        }
        self.state["capture_settings"]["tall_sheet_threshold"] = coverage
            .values()
            .next()
            .map(|c| c["threshold"].clone())
            .unwrap_or(Value::Null);
        self.state["capture_settings"]["tall_sheet_edge_tiles"] = coverage
            .values()
            .next()
            .map(|c| c["edge_tiles"].clone())
            .unwrap_or(Value::Null);
        self.save()
    }
    fn batch_tiles(&self) -> usize {
        self.state["capture_settings"]["batch_tiles"]
            .as_u64()
            .filter(|count| (1..=16).contains(count))
            .unwrap_or(1) as usize
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
    fn images_directory(&self) -> PathBuf {
        if let Some(attempt) = self.state["attempt_id"].as_str() {
            self.directory.join("attempts").join(attempt).join("images")
        } else {
            self.directory.join("images")
        }
    }
    fn image_url(&self, name: &str) -> String {
        if let Some(attempt) = self.state["attempt_id"].as_str() {
            format!("/data/groups/{}/attempts/{attempt}/images/{name}", self.id)
        } else {
            format!("/data/groups/{}/images/{name}", self.id)
        }
    }
    fn record_batch(
        &mut self,
        pixels: u64,
        core_pixels: u64,
        images: usize,
        bytes: u64,
        elapsed: Duration,
        requested: usize,
        actual: usize,
        control_only: bool,
    ) -> Result<()> {
        let old = &self.state["throughput"];
        let total_pixels = old["captured_pixels"].as_u64().unwrap_or(0) + pixels;
        let total_core = old["core_pixels"].as_u64().unwrap_or(0) + core_pixels;
        let total_images = old["uploaded_images"].as_u64().unwrap_or(0) + images as u64;
        let total_bytes = old["png_bytes"].as_u64().unwrap_or(0) + bytes;
        let total_ms = old["capture_elapsed_ms"].as_u64().unwrap_or(0) + elapsed.as_millis() as u64;
        self.state["throughput"] = json!({
            "captured_pixels":total_pixels,"core_pixels":total_core,"uploaded_images":total_images,
            "png_bytes":total_bytes,"capture_elapsed_ms":total_ms,
            "pixels_per_second":if total_ms > 0 {total_pixels as f64 * 1000.0 / total_ms as f64} else {0.0},
            "scope":"physical clients; image capture, upload and host comparison; excludes intentional pause and Excel opening",
            "last_batch":{"captured_pixels":pixels,"core_pixels":core_pixels,"uploaded_images":images,
                "png_bytes":bytes,"elapsed_ms":elapsed.as_millis(),"requested_tiles":requested,"actual_tiles":actual,
                "control_only":control_only,"finished_unix_ms":now_ms()}
        });
        self.save()
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
        for tile in &snapshot.tiles {
            let sheet = sheets.entry(tile.sheet_index).or_insert_with(|| json!({"id":tile.sheet_index.to_string(),"index":tile.sheet_index,"name":tile.sheet_name,"tiles":[]}));
            sheet["tiles"].as_array_mut().unwrap().push(json!({
                "id":format!("s{}-t{}",tile.sheet_index,tile.tile_index),"tile_index":tile.tile_index,
                "address":tile.range,"range":tile.range,"row":tile.core_rows[0],"col":0,
                "core_rows":tile.core_rows,"capture_right_column":tile.capture_right_column,
                "x":tile.x,"y":tile.y,"width":tile.width,"height":tile.height,"status":"pending","images":{},
                "capture_pixels":tile.capture_pixels,"inspection_regions":[]
            }));
            sheet["width"] = json!(sheet["width"]
                .as_u64()
                .unwrap_or(0)
                .max((tile.x + tile.width) as u64));
            sheet["height"] = json!(sheet["height"]
                .as_u64()
                .unwrap_or(0)
                .max((tile.y + tile.height) as u64));
        }
        self.state["sheets"] = json!(sheets.into_values().collect::<Vec<_>>());
        self.state["status"] = json!("capturing");
        self.save()
    }
    fn active(&mut self, tiles: &[Tile]) -> Result<()> {
        if let Some(tile) = tiles.first() {
            self.state["active_sheet"] = json!(tile.sheet_index.to_string());
            self.state["active_tile"] =
                json!(format!("s{}-t{}", tile.sheet_index, tile.tile_index));
        }
        self.state["active_tiles"] = json!(tiles
            .iter()
            .map(|tile| format!("s{}-t{}", tile.sheet_index, tile.tile_index))
            .collect::<Vec<_>>());
        for tile in tiles {
            if let Some(target) = self.tile_mut(tile) {
                target["status"] = json!("capturing");
            }
        }
        self.save()
    }
    fn plan_peers(
        &mut self,
        snapshots: &BTreeMap<String, Snapshot>,
        aliases: &BTreeMap<String, String>,
    ) -> Result<()> {
        let original = &snapshots["original"];
        for sheet in self.state["sheets"].as_array_mut().unwrap() {
            let sheet_index = sheet["index"].as_u64().unwrap() as usize;
            let mut width = 0u32;
            let mut height = 0u32;
            for target in sheet["tiles"].as_array_mut().unwrap() {
                let index = target["tile_index"].as_u64().unwrap() as usize;
                let position = original
                    .tiles
                    .iter()
                    .position(|tile| tile.sheet_index == sheet_index && tile.tile_index == index)
                    .context("planned tile has no original geometry")?;
                let mut peers = BTreeMap::new();
                for (profile, physical) in aliases {
                    let tile = &snapshots[physical].tiles[position];
                    peers.insert(
                        profile.clone(),
                        json!({"x":tile.x,"y":tile.y,"width":tile.width,"height":tile.height,
                        "capture_pixels":tile.capture_pixels}),
                    );
                    width = width.max(tile.x + tile.width);
                    height = height.max(tile.y + tile.height);
                }
                target["planned_images"] = json!(peers);
            }
            sheet["width"] = json!(width);
            sheet["height"] = json!(height);
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
    ) -> Result<()> {
        let name = format!(
            "s{}-t{}-{physical}-preview.png",
            reference.sheet_index, reference.tile_index
        );
        let preview = preview(pixels, 1024);
        save_png(&preview, &self.images_directory().join(&name))?;
        let timestamp = now_ms();
        let url = self.image_url(&name);
        self.state["active_sheet"] = json!(reference.sheet_index.to_string());
        self.state["active_tile"] = json!(format!(
            "s{}-t{}",
            reference.sheet_index, reference.tile_index
        ));
        let metadata = json!({"url":url,"preview_url":url,"pixel_class":format!("{}x{}:{}",tile.width,tile.height,tile.sha256),
            "mask_url":null,"full_url":null,"changed_pixels":null,"width":tile.width,"height":tile.height,
            "preview_width":preview.width(),"preview_height":preview.height(),
            "x":tile.x,"y":tile.y,"status":"received","updated_at":timestamp});
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

    fn tile(&self, tile: &Tile) -> Option<&Value> {
        self.state["sheets"]
            .as_array()?
            .iter()
            .find(|sheet| sheet["index"] == tile.sheet_index)?["tiles"]
            .as_array()?
            .iter()
            .find(|entry| entry["address"] == tile.range)
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

    fn retain_preview(
        &self,
        reference: &Tile,
        physical: &str,
        tile: &Tile,
        pixels: &image::RgbaImage,
        max_width: u32,
    ) -> Result<(u32, u32)> {
        let dimensions = preview_dimensions(pixels, max_width);
        let name = format!(
            "s{}-t{}-{physical}-preview.png",
            reference.sheet_index, reference.tile_index
        );
        let path = self.images_directory().join(&name);
        let url = self.image_url(&name);
        let pixel_class = format!("{}x{}:{}", pixels.width(), pixels.height(), tile.sha256);
        // The URL includes the attempt and physical client. Metadata belongs to
        // this exact source image, so a stale preview cannot stand in for it.
        let recorded = !tile.sha256.is_empty()
            && self
                .tile(reference)
                .and_then(|target| target["images"].as_object())
                .is_some_and(|images| {
                    images.values().any(|image| {
                        image["preview_url"].as_str() == Some(url.as_str())
                            && image["pixel_class"].as_str() == Some(pixel_class.as_str())
                            && image["preview_width"].as_u64() == Some(dimensions.0 as u64)
                            && image["preview_height"].as_u64() == Some(dimensions.1 as u64)
                    })
                });
        if recorded && image::image_dimensions(&path).ok() == Some(dimensions) {
            return Ok(dimensions);
        }
        // Always resize from the full image, never from an earlier preview.
        save_png(&preview(pixels, max_width), &path)?;
        Ok(dimensions)
    }

    // Scientific evidence keeps its exact PNG. Dashboard previews are separate
    // assets and never participate in comparison or the whitespace certificate.
    fn retain_tile(
        &mut self,
        reference: &Tile,
        aliases: &BTreeMap<String, String>,
        images: &BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>,
        keep_for_heights: bool,
    ) -> Result<bool> {
        ensure!(
            aliases.values().all(|client| images.contains_key(client)),
            "cannot classify a tile before every required peer has uploaded"
        );
        let Some((_, _, original)) = images.get("original") else {
            return Ok(true);
        };
        let differing_clients: BTreeSet<_> = images
            .iter()
            .filter_map(|(client, (_, _, pixels))| (pixels != original).then_some(client.as_str()))
            .collect();
        let equal = differing_clients.is_empty();
        // Row-height reports may still need the exact source PNG even when
        // the displayed pixels match. That scientific evidence is independent
        // of the user's resolution setting for equal dashboard tiles.
        let keep_evidence = !equal || keep_for_heights;
        let full = !equal;
        let preview_width = self.preview_width();
        let timestamp = now_ms();
        let prefix = format!("s{}-t{}", reference.sheet_index, reference.tile_index);
        let mut urls = BTreeMap::<String, Value>::new();
        let mut union: Option<image::RgbaImage> = None;
        for (physical, (tile, path, pixels)) in images {
            let basename = format!("{prefix}-{physical}");
            let full_path = self.images_directory().join(format!("{basename}.png"));
            if full {
                fs::copy(path, &full_path)?;
            }
            let max_width = if full { 1024 } else { preview_width };
            let preview_size = self.retain_preview(reference, physical, tile, pixels, max_width)?;
            let full_preview = preview_size == pixels.dimensions();
            let mask_name = format!("{basename}-mask.png");
            let changed = if differing_clients.contains(physical.as_str()) {
                let (mask, changed) = difference_mask(original, pixels);
                save_png(&mask, &self.images_directory().join(&mask_name))?;
                for (x, y, original_pixel) in original.enumerate_pixels() {
                    if x >= pixels.width()
                        || y >= pixels.height()
                        || original_pixel != pixels.get_pixel(x, y)
                    {
                        union
                            .get_or_insert_with(|| {
                                image::RgbaImage::new(original.width(), original.height())
                            })
                            .put_pixel(x, y, image::Rgba([255, 30, 55, 210]));
                    }
                }
                changed
            } else {
                0
            };
            let url = |name: &str| self.image_url(name);
            urls.insert(physical.clone(), json!({
                "url":url(&format!("{basename}-preview.png")),"preview_url":url(&format!("{basename}-preview.png")),
                "full_url":if full {Some(url(&format!("{basename}.png")))} else if full_preview {Some(url(&format!("{basename}-preview.png")))} else {None},
                "mask_url":if changed > 0 {Some(url(&mask_name))} else {None},
                "pixel_class":format!("{}x{}:{}",pixels.width(),pixels.height(),tile.sha256),
                "changed_pixels":changed,"width":pixels.width(),"height":pixels.height(),"x":tile.x,"y":tile.y,
                "preview_width":preview_size.0,"preview_height":preview_size.1,
                "retained_full_resolution":full || full_preview,"updated_at":timestamp,
            }));
        }
        if let Some(union) = union {
            let name = format!("{prefix}-original-mask.png");
            save_png(&union, &self.images_directory().join(&name))?;
            urls.get_mut("original").unwrap()["mask_url"] = json!(self.image_url(&name));
        }
        let logical: BTreeMap<_, _> = aliases
            .iter()
            .filter_map(|(profile, physical)| {
                urls.get(physical)
                    .map(|value| (profile.clone(), value.clone()))
            })
            .collect();
        let regions = if equal {
            Vec::new()
        } else {
            inspection_regions(images, aliases)
        };
        if let Some(tile) = self.tile_mut(reference) {
            tile["status"] = json!(if equal { "equal" } else { "different" });
            tile["equal_across_all"] = json!(equal);
            tile["images"] = json!(logical);
            tile["updated_at"] = json!(timestamp);
            tile["inspection_regions"] = json!(regions);
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

fn preview_dimensions(pixels: &image::RgbaImage, max_width: u32) -> (u32, u32) {
    if max_width == 0 || pixels.width() <= max_width {
        pixels.dimensions()
    } else {
        (
            max_width,
            ((pixels.height() as u64 * max_width as u64) / pixels.width() as u64).max(1) as u32,
        )
    }
}

fn preview(pixels: &image::RgbaImage, max_width: u32) -> image::RgbaImage {
    let dimensions = preview_dimensions(pixels, max_width);
    if dimensions == pixels.dimensions() {
        pixels.clone()
    } else {
        image::imageops::resize(
            pixels,
            dimensions.0,
            dimensions.1,
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

// Inspection regions are views into a scientific capture, never additional
// Excel jobs. Exact region classes let the UI merge only pixel-identical peers.
fn inspection_regions(
    images: &BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>,
    aliases: &BTreeMap<String, String>,
) -> Vec<Value> {
    let Some((_, _, original)) = images.get("original") else {
        return Vec::new();
    };
    let width = images
        .values()
        .map(|(_, _, image)| image.width())
        .max()
        .unwrap_or(0);
    let height = images
        .values()
        .map(|(_, _, image)| image.height())
        .max()
        .unwrap_or(0);
    let columns = width.div_ceil(1024).max(1);
    let rows = height.div_ceil(576).max(1);
    let mut regions = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let x = column * width / columns;
            let y = row * height / rows;
            let right = (column + 1) * width / columns;
            let bottom = (row + 1) * height / rows;
            let mut classes = BTreeMap::new();
            let mut counts = BTreeMap::new();
            let mut changed = vec![false; ((right - x) * (bottom - y)) as usize];
            for (client, (_, _, pixels)) in images {
                let left = x.min(pixels.width());
                let top = y.min(pixels.height());
                let w = right.min(pixels.width()).saturating_sub(left);
                let h = bottom.min(pixels.height()).saturating_sub(top);
                let crop = image::imageops::crop_imm(pixels, left, top, w, h).to_image();
                classes.insert(
                    client,
                    format!("{w}x{h}:{}", artifact_cache::sha256(crop.as_raw())),
                );
                let mut count = 0u64;
                for py in y..bottom {
                    for px in x..right {
                        let before = (px < original.width() && py < original.height())
                            .then(|| original.get_pixel(px, py));
                        let after = (px < pixels.width() && py < pixels.height())
                            .then(|| pixels.get_pixel(px, py));
                        if before != after {
                            count += 1;
                            changed[((py - y) * (right - x) + px - x) as usize] = true;
                        }
                    }
                }
                counts.insert(client, count);
            }
            let pixel_classes: BTreeMap<_, _> = aliases
                .iter()
                .filter_map(|(profile, client)| {
                    classes
                        .get(client)
                        .map(|class| (profile.clone(), class.clone()))
                })
                .collect();
            let changed_pixels_by_profile: BTreeMap<_, _> = aliases
                .iter()
                .filter_map(|(profile, client)| {
                    counts.get(client).map(|count| (profile.clone(), *count))
                })
                .collect();
            regions.push(json!({"id":format!("r{row}-c{column}"),"x":x,"y":y,"width":right-x,"height":bottom-y,
                "changed_pixels":changed.iter().filter(|changed| **changed).count(),"pixel_classes":pixel_classes,"changed_pixels_by_profile":changed_pixels_by_profile,
                "kind":"inspection-region"}));
        }
    }
    regions
}

fn batch_end(
    reference: &Snapshot,
    snapshots: &BTreeMap<String, Snapshot>,
    clients: &BTreeSet<String>,
    start: usize,
    requested: usize,
) -> usize {
    let sheet = reference.tiles[start].sheet_index;
    let mut pixels = BTreeMap::<&String, u64>::new();
    let mut end = start;
    while end < reference.tiles.len()
        && end - start < requested
        && reference.tiles[end].sheet_index == sheet
    {
        if clients.iter().any(|client| {
            let tile = &snapshots[client].tiles[end];
            let capture_pixels = tile
                .capture_pixels
                .max(tile.width as u64 * tile.height as u64);
            pixels.get(client).copied().unwrap_or(0) + capture_pixels > MAX_PENDING_PIXELS
        }) {
            break;
        }
        for client in clients {
            let tile = &snapshots[client].tiles[end];
            *pixels.entry(client).or_default() += tile
                .capture_pixels
                .max(tile.width as u64 * tile.height as u64);
        }
        end += 1;
    }
    end.max(start + 1)
}

impl ResidentCapture {
    pub(super) fn capture_synchronized(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        if std::env::var("EXCEL_SHRINK_CAPTURE_PIPELINE").as_deref() == Ok("0") {
            return self.capture_serialized(clients);
        }
        self.capture_pipelined(clients)
    }

    fn capture_serialized(&mut self, clients: &BTreeSet<String>) -> Result<()> {
        let control_only = !clients.contains("original");
        let reference_client = if control_only {
            clients.first().context("no live clients")?.as_str()
        } else {
            "original"
        };
        let reference = self.snapshot(reference_client)?;
        if !control_only {
            // selected_plan already published complete geometry, including gaps.
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
        let mut start = 0;
        while start < reference.tiles.len() {
            let requested = self.live.as_ref().unwrap().batch_tiles();
            let end = batch_end(&reference, &self.snapshots, clients, start, requested);
            let mut commands = Vec::new();
            for client in clients {
                let snapshot = &self.snapshots[client];
                let capture = snapshot.command.clone();
                let tiles = snapshot.tiles[start..end].to_vec();
                let id = self.issue(
                    client,
                    json!({"kind":"capture_images","capture_id":capture,"tiles":tiles}),
                )?;
                commands.push(PendingCapture {
                    client: client.clone(),
                    capture,
                    command: id,
                    tiles,
                    received: BTreeSet::new(),
                    clock: None,
                    user_pause: Duration::ZERO,
                    sampled: Instant::now(),
                });
            }
            if !control_only {
                let live = self.live.as_mut().unwrap();
                live.state["capture_settings"]["batch_tiles"] = json!(requested);
                live.state["capture_settings"]["effective_batch_tiles"] = json!(end - start);
                live.active(&reference.tiles[start..end])?;
            }
            let mut received =
                BTreeMap::<usize, BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>>::new();
            let mut captured_pixels = 0u64;
            let mut core_pixels = 0u64;
            let mut png_bytes = 0u64;
            let mut elapsed = Duration::ZERO;
            let mut discard_after_result = Vec::new();
            while !commands.is_empty() {
                crate::regression::archive::work_state::event(
                    "waiting_images",
                    "batch_wait",
                    "Waiting for remaining client images or batch responses",
                    false,
                    Some("Office.js image uploads / batch completion"),
                    json!({"clients":commands.iter().map(|job|&job.client).collect::<Vec<_>>() }),
                );
                match self.next_live_event(&mut commands)? {
                    CaptureEvent::Receipt(index, offset, receipt) => {
                        let decoding = Instant::now();
                        let job = &mut commands[index];
                        let mut tile = job.tiles[offset].clone();
                        let metadata: Tile = serde_json::from_value(receipt.clone())?;
                        ensure!(
                            metadata.sheet_index == tile.sheet_index
                                && metadata.tile_index == tile.tile_index
                                && metadata.range == tile.range
                                && metadata.width == tile.width
                                && metadata.height == tile.height
                                && metadata.core_rows == tile.core_rows
                                && metadata.capture_right_column == tile.capture_right_column
                                && metadata.x == tile.x
                                && metadata.y == tile.y
                                && metadata.capture_pixels == tile.capture_pixels,
                            "image-first receipt differs from negotiated tile"
                        );
                        let path = fs::canonicalize(
                            receipt["image_path"].as_str().context("missing PNG path")?,
                        )?;
                        ensure!(
                            path.starts_with(self.directory.join("images")),
                            "image-first PNG outside session"
                        );
                        let bytes = fs::metadata(&path)?.len();
                        ensure!(bytes <= 160 * 1024 * 1024, "image-first PNG too large");
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
                        self.timings.decode_ms += decoding.elapsed().as_secs_f64() * 1000.0;
                        self.timings.images_received += 1;
                        for (key, total) in [
                            ("upload_receive_ms", &mut self.timings.upload_receive_ms),
                            (
                                "upload_validate_publish_ms",
                                &mut self.timings.upload_validate_publish_ms,
                            ),
                        ] {
                            if let Some(value) = receipt["timings"][key]
                                .as_f64()
                                .filter(|value| value.is_finite() && *value >= 0.0)
                            {
                                *total += value;
                            }
                        }
                        crate::regression::archive::work_state::event(
                            "comparing_images",
                            "image_received",
                            "Uploaded image validated and decoded",
                            true,
                            None,
                            json!({"client":job.client,"sheet_index":tile.sheet_index,"sheet_name":tile.sheet_name,"tile_index":tile.tile_index}),
                        );
                        let position = start + offset;
                        self.snapshots.get_mut(&job.client).unwrap().tiles[position] = tile.clone();
                        self.images.insert(
                            (
                                job.client.clone(),
                                job.capture.clone(),
                                tile.sheet_index,
                                tile.tile_index,
                            ),
                            path.clone(),
                        );
                        job.received.insert(tile.tile_index);
                        captured_pixels += tile
                            .capture_pixels
                            .max(tile.width as u64 * tile.height as u64);
                        core_pixels += tile.width as u64 * tile.height as u64;
                        png_bytes += bytes;
                        if !control_only {
                            let previewing = Instant::now();
                            self.live.as_mut().unwrap().received(
                                &reference.tiles[position],
                                &job.client,
                                &tile,
                                &pixels,
                                &self.aliases,
                            )?;
                            self.timings.preview_ms += previewing.elapsed().as_secs_f64() * 1000.0;
                        }
                        let peer_images = received.entry(position).or_default();
                        peer_images.insert(job.client.clone(), (tile, path, pixels));
                        if peer_images.len() == clients.len() {
                            let peer_images = received.remove(&position).unwrap();
                            if !control_only {
                                let comparing = Instant::now();
                                let target = &reference.tiles[position];
                                let keep = self.live.as_mut().unwrap().retain_tile(
                                    target,
                                    &self.aliases,
                                    &peer_images,
                                    height_sheets.contains(&target.sheet_index),
                                )?;
                                self.timings.compare_ms +=
                                    comparing.elapsed().as_secs_f64() * 1000.0;
                                crate::regression::archive::work_state::event(
                                    "comparing_images",
                                    "tile_compared",
                                    "All profile images compared and retained",
                                    true,
                                    None,
                                    json!({"sheet_index":target.sheet_index,"sheet_name":target.sheet_name,"tile_index":target.tile_index}),
                                );
                                if !keep {
                                    for (client, (tile, path, _)) in peer_images {
                                        self.images.remove(&(
                                            client.clone(),
                                            self.snapshots[&client].command.clone(),
                                            tile.sheet_index,
                                            tile.tile_index,
                                        ));
                                        // The host validates each final result against its
                                        // saved PNG. Keep the file until that response exists.
                                        discard_after_result.push(path);
                                    }
                                }
                            }
                        }
                    }
                    CaptureEvent::Response(index) => {
                        let mut job = commands.remove(index);
                        let result = self.wait(&job.client, &job.command)?;
                        ensure!(
                            result["images"]
                                .as_array()
                                .is_some_and(|images| images.len() == job.tiles.len())
                                && job.received.len() == job.tiles.len(),
                            "incomplete image-first batch receipts"
                        );
                        if let Some(clock) = &mut job.clock {
                            elapsed =
                                elapsed.max(clock.active_elapsed().saturating_sub(job.user_pause));
                        }
                    }
                }
            }
            ensure!(
                received.is_empty(),
                "synchronized batch ended without every required peer"
            );
            for path in discard_after_result {
                fs::remove_file(path)?;
            }
            self.live.as_mut().unwrap().state["pipeline_timings"] =
                serde_json::to_value(&self.timings)?;
            self.live.as_mut().unwrap().state["event_transport"] = self.events.snapshot();
            self.live.as_mut().unwrap().record_batch(
                captured_pixels,
                core_pixels,
                (end - start) * clients.len(),
                png_bytes,
                elapsed,
                requested,
                end - start,
                control_only,
            )?;
            start = end;
        }
        if !control_only {
            let live = self.live.as_mut().unwrap();
            live.state["primary_complete"] = json!(true);
            live.state["status"] = json!("comparing");
            live.save()?;
        }
        Ok(())
    }

    fn next_live_event(&mut self, pending: &mut [PendingCapture]) -> Result<CaptureEvent> {
        loop {
            let controls = self.live.as_ref().unwrap().controls();
            if let Some(event) = self.poll_live_event(pending, &controls)? {
                return Ok(event);
            }
            let waiting = Instant::now();
            self.events.wait()?;
            self.timings.wait_ms += waiting.elapsed().as_secs_f64() * 1000.0;
        }
    }

    fn poll_live_event(
        &mut self,
        pending: &mut [PendingCapture],
        controls: &Value,
    ) -> Result<Option<CaptureEvent>> {
        self.check_restart()?;
        let failure = self.directory.join("launcher-error.json");
        ensure!(
            !failure.is_file(),
            "resident Excel launcher failed: {}",
            fs::read_to_string(failure).unwrap_or_default()
        );
        if let Some(status) = self.child.try_wait()? {
            crate::regression::launcher_journal::mark_exited(&mut self.child)?;
            bail!("resident Excel launcher exited ({status}) while waiting for tile uploads")
        }
        let paused = controls["paused"].as_bool().unwrap_or(true);
        let timeout = controls["job_timeout_seconds"]
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or(self.timeout);
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
        if let Some(event) = ready_receipt(&self.directory, pending)? {
            return Ok(Some(event));
        }
        Ok(ready_response(&self.directory, pending).map(CaptureEvent::Response))
    }
}

struct PendingCapture {
    client: String,
    capture: String,
    command: String,
    tiles: Vec<Tile>,
    received: BTreeSet<usize>,
    clock: Option<OpeningPause>,
    user_pause: Duration,
    sampled: Instant,
}

enum CaptureEvent {
    Receipt(usize, usize, Value),
    Response(usize),
}

fn ready_receipt(directory: &Path, pending: &[PendingCapture]) -> Result<Option<CaptureEvent>> {
    let mut earliest: Option<(u64, usize, usize, Value)> = None;
    for (index, job) in pending.iter().enumerate() {
        for (offset, tile) in job.tiles.iter().enumerate() {
            if job.received.contains(&tile.tile_index) {
                continue;
            }
            let path = directory
                .join("receipts")
                .join(&job.client)
                .join(&job.command)
                .join(format!(
                    "s{:04}-t{:05}.json",
                    tile.sheet_index, tile.tile_index
                ));
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let receipt: Value = serde_json::from_slice(&bytes)?;
            let received_at = receipt["received_unix_ms"]
                .as_u64()
                .context("missing upload receipt time")?;
            if earliest
                .as_ref()
                .is_none_or(|(time, _, _, _)| received_at < *time)
            {
                earliest = Some((received_at, index, offset, receipt));
            }
        }
    }
    Ok(earliest.map(|(_, index, offset, receipt)| CaptureEvent::Receipt(index, offset, receipt)))
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
    fn fixture_tile(index: usize, pixels: u64) -> Tile {
        Tile {
            sheet_index: 1,
            sheet_name: "Sheet".into(),
            tile_index: index,
            range: format!("A{}:B{}", index + 1, index + 1),
            core_rows: [index as u32, index as u32 + 1],
            capture_right_column: 1,
            width: 2049,
            height: 577,
            x: 0,
            y: index as u32 * 576,
            capture_pixels: pixels,
            sha256: String::new(),
        }
    }

    #[test]
    fn large_differences_have_exact_inspection_regions_without_new_capture_jobs() {
        let original = image::RgbaImage::from_pixel(2049, 577, image::Rgba([255, 255, 255, 255]));
        let mut variant = original.clone();
        variant.put_pixel(1040, 300, image::Rgba([0, 0, 0, 255]));
        let tile = fixture_tile(0, 2049 * 577);
        let images = BTreeMap::from([
            ("original".into(), (tile.clone(), PathBuf::new(), original)),
            ("maximum-cleaning".into(), (tile, PathBuf::new(), variant)),
        ]);
        let aliases = BTreeMap::from([
            ("original".into(), "original".into()),
            ("default".into(), "original".into()),
            ("maximum-cleaning".into(), "maximum-cleaning".into()),
        ]);
        let regions = inspection_regions(&images, &aliases);
        assert_eq!(regions.len(), 6);
        let differences: Vec<_> = regions
            .iter()
            .filter(|region| region["changed_pixels"] != 0)
            .collect();
        assert_eq!(differences.len(), 1);
        let different = differences[0];
        assert_eq!(different["id"], "r1-c1");
        assert_eq!(different["changed_pixels"], 1);
        assert_eq!(
            different["pixel_classes"]["original"],
            different["pixel_classes"]["default"]
        );
        assert_ne!(
            different["pixel_classes"]["original"],
            different["pixel_classes"]["maximum-cleaning"]
        );
        assert!(regions
            .iter()
            .all(|region| region["kind"] == "inspection-region"));
        assert_eq!(regions.iter().map(|region| region["width"].as_u64().unwrap()*region["height"].as_u64().unwrap()).sum::<u64>(),2049*577);
    }

    #[test]
    fn batch_sizes_preserve_sheet_boundaries_and_each_physical_client_memory_budget() {
        let reference = Snapshot {
            heights: vec![],
            command: "plan".into(),
            tiles: (0..8)
                .map(|index| fixture_tile(index, 16_000_000))
                .collect(),
        };
        let mut variant = reference.clone();
        for tile in &mut variant.tiles {
            tile.capture_pixels = 32_000_000;
        }
        let snapshots = BTreeMap::from([
            ("original".into(), reference.clone()),
            ("variant".into(), variant),
        ]);
        let original = BTreeSet::from(["original".into()]);
        let both = BTreeSet::from(["original".into(), "variant".into()]);
        assert_eq!(batch_end(&reference, &snapshots, &original, 0, 16), 4);
        assert_eq!(batch_end(&reference, &snapshots, &both, 0, 16), 2);
        assert_eq!(batch_end(&reference, &snapshots, &both, 0, 1), 1);
        let mut split = reference.clone();
        split.tiles[1].sheet_index = 2;
        assert_eq!(batch_end(&split, &snapshots, &both, 0, 16), 1);
    }

    #[test]
    fn upload_receipts_are_available_before_the_client_finishes_its_batch() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-batch-receipts")?;
        let tile = fixture_tile(0, 100);
        let mut pending = vec![PendingCapture {
            client: "original".into(),
            capture: "plan".into(),
            command: "batch".into(),
            tiles: vec![tile],
            received: BTreeSet::new(),
            clock: None,
            user_pause: Duration::ZERO,
            sampled: Instant::now(),
        }];
        let path = root.join("receipts/original/batch/s0001-t00000.json");
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(
            path,
            br#"{"received_unix_ms":123,"image_path":"original.png"}"#,
        )?;
        assert!(matches!(
            ready_receipt(&root, &pending)?,
            Some(CaptureEvent::Receipt(0, 0, _))
        ));
        assert_eq!(ready_response(&root, &pending), None);
        pending[0].received.insert(0);
        assert!(ready_receipt(&root, &pending)?.is_none());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn planned_peers_cover_their_real_geometry_and_restarts_use_new_asset_paths() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-planned-peers")?;
        let directory = root.join("groups/fixture");
        let mut live = LiveCapture {
            root: root.clone(),
            directory,
            id: "fixture".into(),
            state: json!({"sheets":[],"attempt_id":"attempt-a"}),
            controls: RefCell::new(json!({"paused":false})),
        };
        fs::create_dir_all(live.images_directory())?;
        let original = Snapshot {
            heights: vec![],
            command: "plan".into(),
            tiles: vec![fixture_tile(0, 100), fixture_tile(1, 100)],
        };
        let mut variant = original.clone();
        variant.tiles[1].y = 800;
        variant.tiles[1].width = 3000;
        let snapshots = BTreeMap::from([
            ("original".into(), original.clone()),
            ("variant".into(), variant),
        ]);
        let aliases = BTreeMap::from([
            ("original".into(), "original".into()),
            ("default".into(), "variant".into()),
        ]);
        live.plan(&original)?;
        live.plan_peers(&snapshots, &aliases)?;
        let sheet = &live.state["sheets"][0];
        assert_eq!(sheet["width"], 3000);
        assert_eq!(sheet["height"], 1377);
        assert_eq!(sheet["tiles"][1]["planned_images"]["default"]["y"], 800);
        assert_eq!(sheet["tiles"][1]["status"], "pending");
        assert_eq!(sheet["tiles"][1]["images"], json!({}));
        let previous = live.images_directory().join("tile.png");
        fs::write(&previous, b"previous evidence")?;
        let previous_url = live.image_url("tile.png");
        live.state["attempt_id"] = json!("attempt-b");
        fs::create_dir_all(live.images_directory())?;
        fs::write(live.images_directory().join("tile.png"), b"new evidence")?;
        assert_ne!(live.image_url("tile.png"), previous_url);
        assert_eq!(fs::read(previous)?, b"previous evidence");
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
            x: 0,
            y: 0,
            capture_pixels: 512,
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
                false
            )
            .is_err());
        assert!(live.retain_tile(&tile, &aliases, &images, false)?);
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
            false
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
                tiles: vec![tile.clone()],
                received: BTreeSet::new(),
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
            x: 0,
            y: 0,
            capture_pixels: 8192,
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
            live.retain_tile(&tile, &aliases, &images, true)?,
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

    struct PreviewFixture {
        root: PathBuf,
        live: LiveCapture,
        tile: Tile,
        aliases: BTreeMap<String, String>,
        images: BTreeMap<String, (Tile, PathBuf, image::RgbaImage)>,
    }

    impl PreviewFixture {
        fn new(pixels: image::RgbaImage) -> Result<Self> {
            let root = artifact_cache::unique_path(&std::env::temp_dir(), "live-preview-reuse")?;
            let mut live = LiveCapture {
                root: root.clone(),
                directory: root.join("groups/fixture"),
                id: "fixture".into(),
                state: json!({"sheets":[],"attempt_id":"first"}),
                controls: RefCell::new(json!({"paused":false,"equal_preview_width":1024})),
            };
            fs::create_dir_all(live.images_directory())?;
            let tile = Tile {
                width: pixels.width(),
                height: pixels.height(),
                sha256: artifact_cache::sha256(pixels.as_raw()),
                ..fixture_tile(0, u64::from(pixels.width()) * u64::from(pixels.height()))
            };
            live.plan(&Snapshot {
                tiles: vec![tile.clone()],
                heights: vec![],
                command: "plan".into(),
            })?;
            let mut fixture = Self {
                root,
                live,
                tile,
                aliases: BTreeMap::new(),
                images: BTreeMap::new(),
            };
            fixture.add_peer("original", pixels)?;
            Ok(fixture)
        }

        fn add_peer(&mut self, name: &str, pixels: image::RgbaImage) -> Result<()> {
            let path = self.root.join(format!("{name}-source.png"));
            pixels.save(&path)?;
            let tile = Tile {
                width: pixels.width(),
                height: pixels.height(),
                sha256: artifact_cache::sha256(pixels.as_raw()),
                ..self.tile.clone()
            };
            self.aliases.insert(name.into(), name.into());
            self.images.insert(name.into(), (tile, path, pixels));
            Ok(())
        }

        fn receive(&mut self) -> Result<()> {
            for (name, (tile, _, pixels)) in &self.images {
                self.live
                    .received(&self.tile, name, tile, pixels, &self.aliases)?;
            }
            Ok(())
        }

        fn retain(&mut self, keep_for_heights: bool) -> Result<bool> {
            self.live
                .retain_tile(&self.tile, &self.aliases, &self.images, keep_for_heights)
        }

        fn asset(&self, name: &str) -> PathBuf {
            self.live
                .images_directory()
                .join(format!("s1-t0-{name}.png"))
        }

        fn image_metadata(&mut self, name: &str) -> &mut Value {
            &mut self.live.state["sheets"][0]["tiles"][0]["images"][name]
        }
    }

    impl Drop for PreviewFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn alternate_png_encoding(path: &Path) -> Result<Vec<u8>> {
        use image::codecs::png::{CompressionType, FilterType, PngEncoder};
        use image::ImageEncoder;

        let before = fs::read(path)?;
        let pixels = image::open(path)?.to_rgba8();
        let mut bytes = Vec::new();
        PngEncoder::new_with_quality(&mut bytes, CompressionType::Best, FilterType::NoFilter)
            .write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        assert_ne!(
            bytes, before,
            "fixture must distinguish reuse from re-encoding"
        );
        fs::write(path, &bytes)?;
        Ok(bytes)
    }

    #[test]
    fn received_preview_is_reused_when_effective_dimensions_match() -> Result<()> {
        let pixels = image::RgbaImage::from_pixel(128, 16, image::Rgba([8, 40, 90, 255]));
        let mut fixture = PreviewFixture::new(pixels.clone())?;
        fixture.add_peer("default", pixels)?;
        fixture.receive()?;
        let original_preview = fixture.asset("original-preview");
        let default_preview = fixture.asset("default-preview");
        let original_bytes = alternate_png_encoding(&original_preview)?;
        let default_bytes = alternate_png_encoding(&default_preview)?;
        for width in [0, 128, 256, 512, 1024] {
            fixture.live.controls.borrow_mut()["equal_preview_width"] = json!(width);
            assert!(!fixture.retain(false)?);
            assert_eq!(fs::read(&original_preview)?, original_bytes);
            assert_eq!(fs::read(&default_preview)?, default_bytes);
            for name in ["original", "default"] {
                let metadata = fixture.image_metadata(name);
                assert_eq!(metadata["preview_width"], 128);
                assert_eq!(metadata["preview_height"], 16);
                assert_eq!(metadata["changed_pixels"], 0);
                assert!(metadata["mask_url"].is_null());
                assert!(!fixture.asset(&format!("{name}-mask")).exists());
            }
        }
        Ok(())
    }

    #[test]
    fn missing_stale_or_wrong_sized_previews_are_regenerated() -> Result<()> {
        for invalid in [
            "missing",
            "source",
            "url",
            "metadata-size",
            "file-size",
            "empty-digest",
        ] {
            let pixels = image::RgbaImage::from_pixel(128, 16, image::Rgba([8, 40, 90, 255]));
            let mut fixture = PreviewFixture::new(pixels.clone())?;
            fixture.receive()?;
            let path = fixture.asset("original-preview");
            let expected = fs::read(&path)?;
            alternate_png_encoding(&path)?;
            match invalid {
                "missing" => fs::remove_file(&path)?,
                "source" => {
                    fixture.image_metadata("original")["pixel_class"] = json!("stale-pixels")
                }
                "url" => {
                    fixture.image_metadata("original")["preview_url"] =
                        json!("/previous-attempt.png")
                }
                "metadata-size" => fixture.image_metadata("original")["preview_width"] = json!(64),
                "file-size" => image::RgbaImage::new(1, 1).save(&path)?,
                "empty-digest" => {
                    fixture.images.get_mut("original").unwrap().0.sha256.clear();
                    fixture.image_metadata("original")["pixel_class"] = json!("128x16:");
                }
                _ => unreachable!(),
            }
            assert!(!fixture.retain(false)?);
            assert_eq!(fs::read(&path)?, expected, "{invalid}");
            assert_eq!(image::open(&path)?.to_rgba8(), pixels);
        }
        Ok(())
    }

    #[test]
    fn preview_reuse_never_crosses_attempt_paths() -> Result<()> {
        let pixels = image::RgbaImage::from_pixel(128, 16, image::Rgba([8, 40, 90, 255]));
        let mut fixture = PreviewFixture::new(pixels)?;
        fixture.receive()?;
        let old_path = fixture.asset("original-preview");
        let expected = fs::read(&old_path)?;
        let old_bytes = alternate_png_encoding(&old_path)?;
        fixture.live.state["attempt_id"] = json!("second");
        fs::create_dir_all(fixture.live.images_directory())?;
        let current_path = fixture.asset("original-preview");
        fs::copy(&old_path, &current_path)?;
        assert!(!fixture.retain(false)?);
        assert_eq!(fs::read(current_path)?, expected);
        assert_eq!(fs::read(old_path)?, old_bytes);
        assert!(fixture.image_metadata("original")["preview_url"]
            .as_str()
            .unwrap()
            .contains("/second/"));
        Ok(())
    }

    #[test]
    fn preview_width_changes_use_full_source_and_preserve_height_evidence() -> Result<()> {
        let pixels = image::RgbaImage::from_fn(2048, 16, |x, y| {
            image::Rgba([(x % 256) as u8, (y * 16) as u8, (x / 16) as u8, 255])
        });
        let mut fixture = PreviewFixture::new(pixels.clone())?;
        fixture.receive()?;
        let source = fixture.images["original"].1.clone();
        let source_bytes = fs::read(&source)?;
        for width in [128, 0, 512, 1024] {
            fixture.live.controls.borrow_mut()["equal_preview_width"] = json!(width);
            assert!(fixture.retain(true)?);
            let expected = preview(&pixels, width);
            assert_eq!(
                image::open(fixture.asset("original-preview"))?.to_rgba8(),
                expected
            );
            assert_eq!(
                fixture.image_metadata("original")["retained_full_resolution"],
                width == 0
            );
            assert_eq!(fs::read(&source)?, source_bytes);
            assert!(!fixture.asset("original").exists());
            assert!(!fixture.asset("original-mask").exists());
        }
        Ok(())
    }

    #[test]
    fn only_different_peers_get_masks_and_original_union_includes_removed_pixels() -> Result<()> {
        let original = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let mut fixture = PreviewFixture::new(original.clone())?;
        fixture.add_peer("default", original.clone())?;
        fixture.add_peer(
            "maximum-cleaning",
            image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])),
        )?;
        let mut changed = original;
        changed.put_pixel(1, 1, image::Rgba([1, 2, 3, 254]));
        fixture.add_peer("too-aggressive-cleaning", changed)?;
        fixture.receive()?;
        assert!(fixture.retain(false)?);
        assert_eq!(fixture.image_metadata("default")["changed_pixels"], 0);
        assert!(fixture.image_metadata("default")["mask_url"].is_null());
        assert!(!fixture.asset("default-mask").exists());
        assert_eq!(
            fixture.image_metadata("maximum-cleaning")["changed_pixels"],
            2
        );
        assert_eq!(
            fixture.image_metadata("too-aggressive-cleaning")["changed_pixels"],
            1
        );
        let union = image::open(fixture.asset("original-mask"))?.to_rgba8();
        assert_eq!(union.pixels().filter(|pixel| pixel[3] != 0).count(), 3);
        assert_ne!(union.get_pixel(2, 0)[3], 0);
        assert_ne!(union.get_pixel(2, 1)[3], 0);
        assert_ne!(union.get_pixel(1, 1)[3], 0);
        for (name, (_, source, _)) in &fixture.images {
            assert_eq!(fs::read(fixture.asset(name))?, fs::read(source)?);
        }
        Ok(())
    }

    #[test]
    fn added_pixels_do_not_create_an_empty_original_mask() -> Result<()> {
        let mut fixture = PreviewFixture::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([1, 2, 3, 255]),
        ))?;
        fixture.add_peer(
            "maximum-cleaning",
            image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255])),
        )?;
        assert!(fixture.retain(false)?);
        assert_eq!(
            fixture.image_metadata("maximum-cleaning")["changed_pixels"],
            2
        );
        assert!(fixture.image_metadata("original")["mask_url"].is_null());
        assert!(!fixture.asset("original-mask").exists());
        Ok(())
    }
}
