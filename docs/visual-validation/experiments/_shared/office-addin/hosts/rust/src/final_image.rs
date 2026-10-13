//! Bounded final PNG export from a known persisted capture, without re-rendering Excel.
use super::*;
use std::time::Instant;

pub(super) async fn finalize(State(state): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    if state.config.experiment != "range-image" {
        return (StatusCode::NOT_FOUND, "Final image export is available for range-image only").into_response();
    }
    let worker_state = state.clone();
    let log_body = body.clone();
    let result = tokio::task::spawn_blocking(move || export(&worker_state, &body)).await;
    match result {
        Ok(Ok(receipt)) => Json(receipt).into_response(),
        other => {
            let message = match other {
                Ok(Err(e)) => format!("{e:#}"),
                Err(e) => format!("final-image worker: {e}"),
                _ => unreachable!(),
            };
            state.record("final_png_failed", json!({"request":log_body,"error":message}));
            (StatusCode::BAD_REQUEST, Json(json!({"error":message,"phase":"final-export"}))).into_response()
        }
    }
}

fn is_redirect(path: &Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return Ok(true); }
    }
    Ok(metadata.file_type().is_symlink())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 94 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn export(state: &Shared, body: &Value) -> Result<Value> {
    // One PNG decode/crop at a time, irrespective of the number of HTTP workers.
    let _guard = state.export_gate.lock().map_err(|_| anyhow!("Export lock poisoned"))?;
    let started = Instant::now();
    let text = |key: &str| -> Result<&str> { body[key].as_str().with_context(|| format!("Missing {key}")) };
    let workbook = text("workbook")?;
    let id = text("source_id")?;
    let sheet = text("sheet")?;
    let address = text("address")?;
    let sha = text("source_sha256")?;
    if !state.expected_set.contains(workbook) || !valid_id(id) || sha.len() != 64 ||
        !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) ||
        sheet.is_empty() || sheet.chars().count() > 31 || address.is_empty() || address.len() > 100 {
        bail!("Invalid final-image identity");
    }
    let number = |key: &str| -> Result<u32> { u32::try_from(body[key].as_u64().with_context(|| format!("Missing {key}"))?).context("Image dimension overflow") };
    let width = number("width")?;
    let sw = number("source_width")?;
    let sh = number("source_height")?;
    let options: Value = serde_json::from_slice(&fs::read(state.config.static_root.join("capture-options.json"))?)?;
    let mp = options["tile_megapixels"].as_u64().unwrap_or(16);
    if ![16,32,64].contains(&mp) || width == 0 || width > sw || sw == 0 || sh == 0 || sw > 30000 || sh > 30000 ||
        u64::from(sw) * u64::from(sh) > mp * 1_000_000 { bail!("Native export pixel budget exceeded"); }
    let dir = state.config.image_root.join(safe_file_name(workbook));
    let source = dir.join(format!("{id}.png"));
    if is_redirect(&dir)? || is_redirect(&source)? {
        bail!("Symbolic paths are not valid image sources");
    }
    if fs::metadata(&source)?.len() > 128 * 1024 * 1024 { bail!("Source PNG byte budget exceeded"); }
    state.record("final_png_started", json!({"workbook":workbook,"sheet":sheet,"address":address,"source_id":id,"width":width,"source_height":sh}));
    let bytes = fs::read(&source)?;
    if format!("{:x}", Sha256::digest(&bytes)) != sha { bail!("Source PNG hash changed"); }
    let dimensions = ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png).into_dimensions()?;
    if dimensions != (sw, sh) { bail!("Source PNG dimensions differ from the capture receipt"); }
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
    let cropped = image::imageops::crop_imm(&decoded, 0, 0, width, sh).to_image();
    drop(decoded);
    let output_id = format!("{id}-final");
    let destination = dir.join(format!("{output_id}.png"));
    if destination.exists() { bail!("Final artifact already exists"); }
    let temporary = destination.with_extension("png.new");
    let mut created_temporary = false;
    let result = (|| -> Result<Value> {
        let mut handle = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        created_temporary = true;
        image::DynamicImage::ImageRgba8(cropped).write_to(&mut handle, image::ImageFormat::Png)?;
        handle.flush()?; handle.sync_all()?; drop(handle);
        let output_bytes = fs::read(&temporary)?;
        fs::rename(&temporary, &destination)?;
        let receipt = json!({"capture":{"id":output_id,"workbook":workbook,"sheet":sheet,"address":address,
            "relative_png":format!("{}/{}.png",safe_file_name(workbook),output_id),"png_width":width,"png_height":sh,
            "png_bytes":output_bytes.len(),"sha256":format!("{:x}",Sha256::digest(&output_bytes)),"source_id":id,"export_kind":"native-right-crop"},
            "crop_ms":started.elapsed().as_secs_f64()*1000.0});
        state.record("final_png_saved", receipt.clone());
        Ok(receipt)
    })();
    if result.is_err() && created_temporary { let _ = fs::remove_file(&temporary); }
    result
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ids_reject_path_traversal() { for id in ["", "../x", "x/y", "x\\y", "x.png", "x:y"] { assert!(!valid_id(id)); } assert!(valid_id("c00005-p1-t000001")); }
    #[test] fn crop_preserves_pixels() {
        let im = RgbaImage::from_fn(7, 5, |x,y| Rgba([x as u8,y as u8,31,255]));
        let cut = image::imageops::crop_imm(&im,0,0,3,5).to_image();
        assert_eq!(cut.dimensions(),(3,5));
        for y in 0..5 {for x in 0..3 {assert_eq!(im.get_pixel(x,y),cut.get_pixel(x,y));}}
    }
}
