use super::*;
use crate::regression::visual_compare::{compare_tile_images, ImageDifference};
use std::io::{BufReader, Read, Write};
use std::process::Stdio;

const MAX_PNG_BYTES: usize = 160 * 1024 * 1024;

pub(in crate::regression) fn compare_paired_images(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
) -> Result<(Vec<PairedImageTile>, Vec<ImageDifference>)> {
    match run_pair(plan, original, shrunk, output, rows, 32) {
        Err(error) if error.downcast_ref::<ImageBudgetError>().is_some() => {
            run_pair(plan, original, shrunk, output, rows, 16)
        }
        result => result,
    }
}

fn run_pair(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    rows: Option<&BTreeMap<usize, Vec<u32>>>,
    megapixels: u32,
) -> Result<(Vec<PairedImageTile>, Vec<ImageDifference>)> {
    fs::create_dir_all(output)?;
    let selected = selected_sheets(plan, rows)?;
    let side = |original_side: bool| -> Value {
        let sheets: Vec<_> = selected
            .iter()
            .map(|sheet| {
                let mut config = if original_side {
                    sheet.original_config.clone()
                } else {
                    sheet.shrunk_config.clone()
                };
                if let Some(rows) = rows {
                    config["rows"] = json!(rows[&sheet.sheet_index]);
                }
                config
            })
            .collect();
        json!({"mode":"paired", "tile_megapixels":megapixels,
            "workbook":if original_side { &plan.original_workbook } else { &plan.shrunk_workbook }, "sheets":sheets})
    };
    let config = output.join("config.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&json!({"mode":"paired",
        "sheets":selected.iter().map(|s| json!({"index":s.sheet_index,"name":s.sheet_name})).collect::<Vec<_>>(),
        "original":side(true), "shrunk":side(false)}))?,
    )?;
    let transfer = CaptureTransfer::new(original, &config)?;
    fs::copy(shrunk, transfer.0.join("comparison.xlsx"))?;
    let script = super::super::repo_root().join("excel-shrink-cli/office-capture/run.ps1");
    let pipe = format!(
        "es-pair-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut child = Command::new("powershell.exe")
        .args([
            "-STA",
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(windows_path(&script)?)
        .arg("-WorkbookPath")
        .arg(windows_path(&transfer.0.join("workbook.xlsx"))?)
        .arg("-ComparisonWorkbookPath")
        .arg(windows_path(&transfer.0.join("comparison.xlsx"))?)
        .arg("-ConfigPath")
        .arg(windows_path(&transfer.0.join("config.json"))?)
        .arg("-OutputDir")
        .arg(windows_path(&transfer.0.join("output"))?)
        .arg("-PublishedOutputDir")
        .arg(windows_path(output)?)
        .arg("-StreamPipe")
        .arg(&pipe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start paired worksheet capture")?;
    let stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut message = Vec::new();
        // Drain all diagnostics to avoid blocking the child, retain only a short error.
        let mut reader = BufReader::new(stderr);
        let mut buffer = [0; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let keep = count.min(8000usize.saturating_sub(message.len()));
            message.extend_from_slice(&buffer[..keep]);
        }
        message
    });
    let mut input = child.stdin.take().unwrap();
    let mut stream = BufReader::new(child.stdout.take().unwrap());
    let mut tiles = Vec::new();
    let mut differences = Vec::new();
    let mut metadata = ImageTiles::new();
    let mut consumer_error = None;
    loop {
        let frame = read_frame(&mut stream);
        let Some((info, left, right)) = (match frame {
            Ok(frame) => frame,
            Err(error) => {
                consumer_error.get_or_insert(error);
                break;
            }
        }) else {
            break;
        };
        let compared = (|| -> Result<()> {
            let sheet = selected
                .iter()
                .find(|s| s.sheet_index == info.sheet_index)
                .context("unexpected streamed worksheet")?;
            ensure!(
                info.sheet_name == sheet.sheet_name,
                "streamed sheet name differs"
            );
            let (first, last, start_column, end_column) = parse_range(&info.range)?;
            ensure!(
                first >= 1
                    && last <= sheet.bottom_row + 1
                    && start_column == 1
                    && end_column == sheet.capture_right_column + 1,
                "streamed range exceeds the worksheet plan"
            );
            ensure!(
                metadata
                    .insert((info.sheet_index, info.tile_index), info.clone())
                    .is_none(),
                "duplicate streamed image"
            );
            let mut tile = PairedImageTile {
                sheet_index: info.sheet_index,
                sheet_name: info.sheet_name,
                range: info.range,
                tile_index: info.tile_index,
                original_image: PathBuf::new(),
                shrunk_image: PathBuf::new(),
            };
            if rows.is_some() {
                let evidence = output.join("height-evidence");
                fs::create_dir_all(&evidence)?;
                tile.original_image = evidence.join(format!(
                    "s{}-t{}-original.png",
                    tile.sheet_index, tile.tile_index
                ));
                tile.shrunk_image = evidence.join(format!(
                    "s{}-t{}-shrunk.png",
                    tile.sheet_index, tile.tile_index
                ));
                fs::write(&tile.original_image, &left)?;
                fs::write(&tile.shrunk_image, &right)?;
            } else {
                let a = decode(&left)?;
                let b = decode(&right)?;
                differences.extend(compare_tile_images(
                    &tile,
                    &a,
                    &b,
                    &output.join("differences"),
                )?);
            }
            tiles.push(tile);
            Ok(())
        })();
        input.write_all(&[if compared.is_ok() { 1 } else { 0 }])?;
        input.flush()?;
        if let Err(error) = compared {
            consumer_error = Some(error);
        }
    }
    if consumer_error.is_some() {
        // Reject the current pair and drain the relay so the launcher's cleanup
        // can finish even if a frame was incomplete or its metadata was invalid.
        let _ = input.write_all(&[0]);
        let _ = input.flush();
    }
    drop(input);
    if consumer_error.is_some() {
        let _ = std::io::copy(&mut stream, &mut std::io::sink());
    }
    let status = child.wait()?;
    let stderr = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
    transfer.publish(output)?;
    if let Ok(bytes) = fs::read(output.join("result.json")) {
        let result: OfficeResult = serde_json::from_slice(&bytes)?;
        if let Some(error) = result.error {
            if error.code == "IMAGE_BUDGET" {
                return Err(ImageBudgetError(error.message).into());
            }
            bail!("paired worksheet capture failed: {}", error.message);
        }
    } else {
        bail!("paired capture produced no result ({status}): {stderr}");
    }
    if let Some(error) = consumer_error {
        return Err(error);
    }
    ensure!(
        status.success(),
        "paired capture failed ({status}): {stderr}"
    );
    validate_tile_coverage(&metadata, &selected, rows)?;
    Ok((tiles, differences))
}

fn decode(bytes: &[u8]) -> Result<image::RgbaImage> {
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let (width, height) = reader.into_dimensions()?;
    ensure!(
        width > 0
            && height > 0
            && width <= 30_000
            && height <= 30_000
            && u64::from(width) * u64::from(height) <= 32_000_000,
        "streamed PNG exceeds image bounds"
    );
    Ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.to_rgba8())
}

fn read_frame(reader: &mut impl Read) -> Result<Option<(ImageTile, Vec<u8>, Vec<u8>)>> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .context("paired image stream ended before completion")?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 {
        return Ok(None);
    }
    ensure!(
        (12..=2 * MAX_PNG_BYTES + 65536).contains(&length),
        "invalid paired frame length"
    );
    let mut sizes = [0; 12];
    reader.read_exact(&mut sizes)?;
    let size = |offset| u32::from_le_bytes(sizes[offset..offset + 4].try_into().unwrap()) as usize;
    let (meta, left, right) = (size(0), size(4), size(8));
    ensure!(
        meta <= 65536
            && left <= MAX_PNG_BYTES
            && right <= MAX_PNG_BYTES
            && 12 + meta + left + right == length,
        "invalid paired frame sizes"
    );
    let mut metadata = vec![0; meta];
    reader.read_exact(&mut metadata)?;
    let mut info: Value = serde_json::from_slice(&metadata)?;
    info["image_path"] = json!("");
    let info = serde_json::from_value(info)?;
    let mut a = vec![0; left];
    let mut b = vec![0; right];
    reader.read_exact(&mut a)?;
    reader.read_exact(&mut b)?;
    Ok(Some((info, a, b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receives_exact_png_bytes_and_rejects_truncated_pairs() -> Result<()> {
        let info = json!({"sheet_index":1,"sheet_name":"S","tile_index":0,"range":"A1:B2"});
        let metadata = serde_json::to_vec(&info)?;
        let mut frame = Vec::new();
        for size in [12 + metadata.len() + 5, metadata.len(), 2, 3] {
            frame.extend_from_slice(&(size as u32).to_le_bytes());
        }
        frame.extend_from_slice(&metadata);
        frame.extend_from_slice(b"abcde");
        let (_, original, shrunk) = read_frame(&mut frame.as_slice())?.unwrap();
        assert_eq!(original, b"ab");
        assert_eq!(shrunk, b"cde");
        assert!(read_frame(&mut &frame[..frame.len() - 1]).is_err());
        Ok(())
    }

    #[test]
    fn rejects_incomplete_and_oversized_image_frames() {
        assert!(read_frame(&mut &b""[..]).is_err());
        assert!(read_frame(&mut &u32::MAX.to_le_bytes()[..]).is_err());
        assert!(read_frame(&mut &0u32.to_le_bytes()[..]).unwrap().is_none());
    }
}
