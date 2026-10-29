#[path = "paired.rs"]
mod paired;
#[path = "classic_telemetry.rs"]
mod telemetry;

use super::{read_json, serve, with_web, write_durable_bytes, ApiError, HttpResult};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Query, State},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use paired::PairedCapture;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;

const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 120_000_000;

struct Host {
    config: Value,
    mode: String,
    output: PathBuf,
    published_output: PathBuf,
    run_id: String,
    sheets: BTreeMap<i64, String>,
    paired: Option<PairedCapture>,
    results: Mutex<HashMap<String, Value>>,
    config_seen: AtomicBool,
    original_seen: AtomicBool,
    shrunk_seen: AtomicBool,
    telemetry: Option<Mutex<telemetry::Telemetry>>,
    telemetry_write_failed: AtomicBool,
}

impl Host {
    async fn observe(&self, data: &Value) -> Result<bool> {
        if let Some(telemetry) = &self.telemetry {
            let mut telemetry = telemetry.lock().await;
            telemetry.observe(data)?;
            return Ok(self.publish_observations(&telemetry).await);
        }
        Ok(false)
    }

    async fn publish_observations(&self, telemetry: &telemetry::Telemetry) -> bool {
        if self.telemetry_write_failed.load(Ordering::Relaxed) {
            return false;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(2), telemetry.publish()).await {
            Ok(Ok(())) => true,
            result => {
                // Optional diagnostics cannot hold up or fail a scientific
                // comparison. Stop retrying a broken destination in this host.
                eprintln!("Classic telemetry publishing disabled: {result:?}");
                self.telemetry_write_failed.store(true, Ordering::Relaxed);
                false
            }
        }
    }

    fn result_path(&self) -> PathBuf {
        self.output.join("result.json")
    }

    async fn publish_result(&self, result: &Value) -> Result<()> {
        write_durable_bytes(&self.result_path(), &serde_json::to_vec(result)?, false).await
    }

    fn paired(&self) -> Result<&PairedCapture> {
        self.paired.as_ref().context("Not a paired capture.")
    }

    fn check_sheet(&self, data: &Value) -> Result<i64> {
        let index = data["sheet_index"]
            .as_i64()
            .context("Missing sheet index.")?;
        ensure!(
            self.sheets
                .get(&index)
                .is_some_and(|name| Some(name.as_str()) == data["sheet_name"].as_str()),
            "Unknown worksheet."
        );
        Ok(index)
    }
}

pub async fn run(args: Vec<OsString>) -> Result<()> {
    let args = arguments(args)?;
    let config_path = absolute(&args, "config")?;
    let output = absolute(&args, "output")?;
    let published_output = absolute(&args, "published-output")?;
    let web = absolute(&args, "web")?;
    let run_id = argument(&args, "run-id")?.to_owned();
    ensure!(
        config_path.is_file() && web.is_dir(),
        "Capture config or web assets are missing."
    );
    tokio::fs::create_dir_all(&output).await?;
    let capture_config = read_json(&config_path).await?;
    let mode = required_string(&capture_config, "mode")?.to_owned();
    ensure!(
        matches!(mode.as_str(), "heights" | "capture" | "paired"),
        "Invalid capture mode."
    );
    let mut sheets = BTreeMap::new();
    let mut sheet_order = Vec::new();
    for sheet in capture_config["sheets"]
        .as_array()
        .context("Capture config has no sheets array.")?
    {
        let index = sheet["index"].as_i64().context("Missing sheet index.")?;
        let name = required_string(sheet, "name")?.to_owned();
        ensure!(
            index > 0 && sheets.insert(index, name).is_none(),
            "Invalid or duplicate sheet index."
        );
        sheet_order.push(index);
    }
    let paired = if mode == "paired" {
        Some(PairedCapture::new(
            argument(&args, "stream-pipe")?,
            run_id.clone(),
            sheet_order,
        )?)
    } else {
        None
    };
    let port: u16 = args
        .get("port")
        .map(|_| argument(&args, "port"))
        .transpose()?
        .unwrap_or("3000")
        .parse()?;
    ensure!(port >= 1024, "Invalid localhost port.");
    let telemetry = if args.contains_key("telemetry-path") {
        Some(Mutex::new(telemetry::Telemetry::new(
            absolute(&args, "telemetry-path")?,
            argument(&args, "attempt-id")?,
            &run_id,
            argument(&args, "comparison-profile")?,
        )))
    } else {
        None
    };
    let host = Arc::new(Host {
        config: capture_config,
        mode,
        output,
        published_output,
        run_id,
        sheets,
        paired,
        results: Mutex::new(HashMap::new()),
        config_seen: AtomicBool::new(false),
        original_seen: AtomicBool::new(false),
        shrunk_seen: AtomicBool::new(false),
        telemetry,
        telemetry_write_failed: AtomicBool::new(false),
    });
    if let Some(telemetry) = &host.telemetry {
        // Clear the preceding profile's snapshot before accepting clients for
        // this pair. Its new run id also rejects delayed old observations.
        let observation = telemetry.lock().await;
        host.publish_observations(&observation).await;
    }
    let routes = Router::new()
        .route("/health", get(health))
        .route("/config", get(config))
        .route(
            "/telemetry",
            post(observation).layer(DefaultBodyLimit::max(8192)),
        )
        .route("/pair/geometry", post(geometry))
        .route("/pair/heights", post(heights))
        .route("/pair/sheet", post(sheet_done))
        .route("/pair/image", post(paired_image))
        .route("/image", post(image))
        .route("/result", post(result))
        .with_state(host);
    serve(with_web(routes, &web, MAX_IMAGE_BYTES * 2), port).await
}

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(json!({
        "ok":true, "run_id":host.run_id,
        "config_seen":host.config_seen.load(Ordering::Relaxed),
        "original_seen":host.original_seen.load(Ordering::Relaxed),
        "shrunk_seen":host.shrunk_seen.load(Ordering::Relaxed),
        "telemetry_available":host.telemetry.is_some() && !host.telemetry_write_failed.load(Ordering::Relaxed)
    }))
}

async fn observation(
    State(host): State<Arc<Host>>,
    Json(data): Json<Value>,
) -> HttpResult<Json<Value>> {
    if serde_json::to_vec(&data)?.len() > 8192 {
        return Err(ApiError::bad_request(
            "Classic observation exceeds the metadata budget.",
        ));
    }
    let saved = host.observe(&data).await?;
    Ok(Json(json!({"ok":true,"telemetry_saved":saved})))
}

async fn config(
    State(host): State<Arc<Host>>,
    Query(query): Query<HashMap<String, String>>,
) -> HttpResult<Json<Value>> {
    host.config_seen.store(true, Ordering::Relaxed);
    let mut response = host.config.clone();
    if let Some(paired) = &host.paired {
        let side = query.get("side").map(String::as_str).unwrap_or_default();
        match side {
            "original" => host.original_seen.store(true, Ordering::Relaxed),
            "shrunk" => host.shrunk_seen.store(true, Ordering::Relaxed),
            _ => return Err(ApiError::bad_request("Invalid capture side.")),
        }
        if !paired.start_client(side).await? {
            let _gate = host.results.lock().await;
            if !host.result_path().exists() {
                let result = json!({
                    "run_id":host.run_id, "heights":[], "captures":[],
                    "error":{"code":"TASKPANE_RESTARTED", "message":format!("The {side} capture task pane restarted; discard this workbook session.")}
                });
                host.publish_result(&result).await?;
                paired.finish(true).await?;
            }
            return Err(ApiError::conflict("Capture task pane restarted."));
        }
        response = response[side].clone();
        ensure_object(&response)?;
        response["mode"] = json!("paired");
        response["side"] = json!(side);
    }
    // The launcher releases captures only after error indicators are disabled
    // in both exact, owned Excel instances. Health flags above remain visible.
    if let Some(ready) = host.config["_capture_ready_path"].as_str() {
        let timeout = host.config["_capture_ready_timeout_seconds"].as_u64().unwrap_or(600).clamp(1, 86400);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
        while !Path::new(ready).is_file() {
            if tokio::time::Instant::now() >= deadline {
                return Err(ApiError::conflict("Excel capture configuration was not confirmed by its launcher."));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    if let Some(fields) = response.as_object_mut() {
        fields.remove("_capture_ready_path");
        fields.remove("_capture_ready_timeout_seconds");
    }
    response["run_id"] = json!(host.run_id);
    response["telemetry"] =
        json!(host.telemetry.is_some() && !host.telemetry_write_failed.load(Ordering::Relaxed));
    Ok(Json(response))
}

async fn geometry(
    State(host): State<Arc<Host>>,
    Json(data): Json<Value>,
) -> HttpResult<Json<Value>> {
    host.paired()?.validate(&data)?;
    host.observe(
        &json!({"run_id":host.run_id,"side":data["side"],"state":"waiting_for_peers",
        "message":"Geometry measured; waiting for the other workbook's capture boundaries",
        "kind":"geometry_received","progress":true,"wait_reason":"paired geometry",
        "details":{"sheet_index":data["sheet_index"],"sheet_name":data["sheet_name"]}}),
    )
    .await?;
    Ok(Json(host.paired()?.geometry(data).await?))
}

async fn heights(
    State(host): State<Arc<Host>>,
    Json(data): Json<Value>,
) -> HttpResult<Json<Value>> {
    host.paired()?.validate(&data)?;
    host.observe(
        &json!({"run_id":host.run_id,"side":data["side"],"state":"waiting_for_peers",
        "message":"Row heights received; waiting for the paired row-height comparison",
        "kind":"heights_received","progress":true,"wait_reason":"paired row heights",
        "details":{"sheet_index":data["sheet_index"],"sheet_name":data["sheet_name"]}}),
    )
    .await?;
    Ok(Json(host.paired()?.heights(data).await?))
}

async fn sheet_done(
    State(host): State<Arc<Host>>,
    Json(data): Json<Value>,
) -> HttpResult<Json<Value>> {
    host.paired()?.sheet_done(data).await?;
    Ok(Json(json!({"ok":true})))
}

async fn paired_image(
    State(host): State<Arc<Host>>,
    Json(mut data): Json<Value>,
) -> HttpResult<Json<Value>> {
    let pair = host.paired()?;
    pair.validate(&data)?;
    host.check_sheet(&data)?;
    let bytes = decode_image(&data)?;
    let (width, height) = check_png(&bytes)?;
    if width > 30_000 || height > 30_000 || u64::from(width) * u64::from(height) > 64_000_000 {
        return Err(ApiError::bad_request(
            "Paired image exceeds the tile budget.",
        ));
    }
    data.as_object_mut()
        .context("Image payload must be an object.")?
        .remove("base64");
    host.observe(
        &json!({"run_id":host.run_id,"side":data["side"],"state":"waiting_for_peers",
        "message":"Captured image received; waiting for its peer and host pixel comparison",
        "kind":"image_received","progress":true,"wait_reason":"paired image comparison",
        "details":{"sheet_index":data["sheet_index"],"sheet_name":data["sheet_name"],
            "range":data["range"],"tile_index":data["tile_index"]}}),
    )
    .await?;
    pair.image(data, bytes).await?;
    Ok(Json(json!({"ok":true})))
}

async fn image(State(host): State<Arc<Host>>, Json(data): Json<Value>) -> HttpResult<Json<Value>> {
    if host.mode != "capture" {
        return Err(ApiError::bad_request(
            "Images are not accepted in height mode.",
        ));
    }
    if data["run_id"].as_str() != Some(&host.run_id) {
        return Err(ApiError::conflict("Capture run changed."));
    }
    let index = host.check_sheet(&data)?;
    let range = required_string(&data, "range")?;
    if !valid_range(range) {
        return Err(ApiError::bad_request("Invalid capture range."));
    }
    let tile = data["tile_index"].as_i64().context("Missing tile index.")?;
    if !(0..=1_000_000).contains(&tile) {
        return Err(ApiError::bad_request("Invalid tile index."));
    }
    let bytes = decode_image(&data)?;
    let (width, height) = check_png(&bytes)?;
    let relative = Path::new("images").join(format!("s{index:04}-t{tile:05}.png"));
    write_durable_bytes(&host.output.join(&relative), &bytes, false).await?;
    Ok(Json(
        json!({"path":host.published_output.join(relative), "width":width, "height":height}),
    ))
}

async fn result(State(host): State<Arc<Host>>, Json(data): Json<Value>) -> HttpResult<Json<Value>> {
    if data["run_id"].as_str() != Some(&host.run_id) {
        return Err(ApiError::conflict("Capture run changed."));
    }
    let mut results = host.results.lock().await;
    if host.result_path().exists() {
        return Err(ApiError::conflict("Result already received."));
    }
    if let Some(pair) = &host.paired {
        pair.validate(&data)?;
        let side = required_string(&data, "side")?.to_owned();
        if results.contains_key(&side) {
            return Err(ApiError::conflict("Duplicate paired result."));
        }
        let error = data["error"].clone();
        results.insert(side, data);
        if !error.is_null() || results.len() == 2 {
            if error.is_null() && !pair.complete().await {
                return Err(ApiError::bad_request(
                    "Paired capture has incomplete worksheets.",
                ));
            }
            let mut output = json!({"run_id":host.run_id, "heights":[], "captures":[]});
            if !error.is_null() {
                output["error"] = error.clone();
            }
            host.publish_result(&output).await?;
            pair.finish(!error.is_null()).await?;
        }
    } else {
        let field = if host.mode == "heights" {
            "heights"
        } else {
            "captures"
        };
        if !data[field].is_array() {
            return Err(ApiError::bad_request(format!(
                "Result has no {field} array."
            )));
        }
        host.publish_result(&data).await?;
    }
    Ok(Json(json!({"ok":true})))
}

fn arguments(args: Vec<OsString>) -> Result<HashMap<String, OsString>> {
    ensure!(args.len() % 2 == 0, "Expected named host arguments.");
    let mut result = HashMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0]
            .to_str()
            .and_then(|s| s.strip_prefix("--"))
            .context("Invalid host argument.")?;
        ensure!(
            result.insert(key.to_owned(), pair[1].clone()).is_none(),
            "Duplicate host argument."
        );
    }
    Ok(result)
}

fn argument<'a>(args: &'a HashMap<String, OsString>, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("Missing --{key}."))
}

fn absolute(args: &HashMap<String, OsString>, key: &str) -> Result<PathBuf> {
    let value = args.get(key).with_context(|| format!("Missing --{key}."))?;
    ensure!(!value.is_empty(), "Missing --{key}.");
    Ok(std::path::absolute(value)?)
}

fn required_string<'a>(data: &'a Value, field: &str) -> Result<&'a str> {
    data[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Missing {field}."))
}

fn ensure_object(value: &Value) -> Result<()> {
    ensure!(
        value.is_object(),
        "Capture configuration must be an object."
    );
    Ok(())
}

fn valid_range(range: &str) -> bool {
    fn cell(value: &str) -> bool {
        let bytes = value.as_bytes();
        let letters = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
        letters > 0
            && letters < bytes.len()
            && (b'1'..=b'9').contains(&bytes[letters])
            && bytes[letters..].iter().all(u8::is_ascii_digit)
    }
    let parts: Vec<_> = range.split(':').collect();
    range.len() <= 32 && parts.len() <= 2 && parts.into_iter().all(cell)
}

fn decode_image(data: &Value) -> Result<Vec<u8>> {
    let encoded = required_string(data, "base64")?;
    const PREFIX: &str = "data:image/png;base64,";
    let encoded = if encoded
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
    {
        &encoded[PREFIX.len()..]
    } else {
        encoded
    };
    ensure!(
        encoded.len() <= MAX_IMAGE_BYTES * 4 / 3 + 4,
        "Image is too large."
    );
    let bytes = STANDARD.decode(encoded)?;
    ensure!(bytes.len() <= MAX_IMAGE_BYTES, "Image is too large.");
    Ok(bytes)
}

fn check_png(png: &[u8]) -> Result<(u32, u32)> {
    ensure!(
        png.len() >= 45 && png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "Image is not a PNG."
    );
    let mut offset = 8;
    let mut dimensions = None;
    let mut image_data_seen = false;
    while offset + 12 <= png.len() {
        let length = u32::from_be_bytes(png[offset..offset + 4].try_into()?) as usize;
        ensure!(length <= png.len() - offset - 12, "Truncated PNG chunk.");
        let chunk = &png[offset + 4..offset + 8 + length];
        let expected_crc =
            u32::from_be_bytes(png[offset + 8 + length..offset + 12 + length].try_into()?);
        ensure!(
            crc32fast::hash(chunk) == expected_crc,
            "Invalid PNG checksum."
        );
        let kind = &png[offset + 4..offset + 8];
        if offset == 8 {
            ensure!(kind == b"IHDR" && length == 13, "Missing PNG header.");
            let width = u32::from_be_bytes(png[offset + 8..offset + 12].try_into()?);
            let height = u32::from_be_bytes(png[offset + 12..offset + 16].try_into()?);
            ensure!(
                width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS,
                "Invalid PNG dimensions."
            );
            ensure!(
                png[offset + 18..offset + 21] == [0, 0, 0],
                "Unsupported PNG encoding."
            );
            dimensions = Some((width, height));
        } else {
            ensure!(kind != b"IHDR", "Duplicate PNG header.");
        }
        if kind == b"IDAT" {
            image_data_seen = true;
        }
        if kind == b"IEND" {
            ensure!(
                length == 0 && offset + 12 == png.len(),
                "Invalid PNG ending."
            );
            ensure!(image_data_seen, "Incomplete PNG.");
            return dimensions.context("Missing PNG header.");
        }
        offset += length + 12;
    }
    bail!("Incomplete PNG.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_checks_crc_and_complete_evidence() {
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(check_png(&image).unwrap(), (1, 1));
        let mut corrupt = image.clone();
        corrupt[29] ^= 1;
        assert!(check_png(&corrupt).is_err());
        assert!(check_png(&image[..image.len() - 1]).is_err());
        image.push(0);
        assert!(check_png(&image).is_err());
    }

    #[test]
    fn ranges_and_tile_identity_do_not_accept_paths() {
        for valid in ["A1", "AA10:XFD1048576"] {
            assert!(valid_range(valid));
        }
        for invalid in ["A0", "a1", "A1:B2:C3", "../A1", "A1\n"] {
            assert!(!valid_range(invalid));
        }
    }

    use image::ImageEncoder;
}
