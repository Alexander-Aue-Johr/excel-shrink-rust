use super::*;
use crate::regression::visual_compare::{compare_tile_images, ImageDifference};
use std::io::{BufReader, Read, Write};
use std::process::Stdio;

const MAX_PNG_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug)]
struct TaskpaneRestarted(String);

impl fmt::Display for TaskpaneRestarted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "capture task pane restarted: {}", self.0)
    }
}

impl std::error::Error for TaskpaneRestarted {}

fn retry_with_smaller_tiles(error: &anyhow::Error) -> bool {
    error.is::<ImageBudgetError>() || error.is::<TaskpaneRestarted>()
}

pub(in crate::regression) struct PairedVisualResult {
    pub tiles: Vec<PairedImageTile>,
    pub differences: Vec<ImageDifference>,
    pub heights: Vec<RowHeightDifference>,
}

pub(in crate::regression) fn compare_paired_visuals(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
) -> Result<PairedVisualResult> {
    match run_pair(plan, original, shrunk, output, 32) {
        Err(error) if retry_with_smaller_tiles(&error) => {
            run_pair(plan, original, shrunk, output, 16)
        }
        result => result,
    }
}

fn run_pair(
    plan: &PairedPlan,
    original: &Path,
    shrunk: &Path,
    output: &Path,
    megapixels: u32,
) -> Result<PairedVisualResult> {
    fs::create_dir_all(output)?;
    let previous_result = output.join("result.json");
    if previous_result.exists() {
        fs::remove_file(previous_result)?;
    }
    let selected = selected_sheets(plan, None)?;
    let side = |original_side: bool| -> Value {
        let sheets: Vec<_> = selected
            .iter()
            .map(|sheet| {
                let config = if original_side {
                    sheet.original_config.clone()
                } else {
                    sheet.shrunk_config.clone()
                };
                config
            })
            .collect();
        json!({"mode":"paired", "check_heights":true, "tile_megapixels":megapixels,
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
    let script = crate::validation::capture::capture_directory()?.join("run.ps1");
    let pipe = format!(
        "es-pair-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-STA",
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(windows_path(&script)?)
        .arg("-HostExecutable")
        .arg(windows_path(&crate::capture_host::executable()?)?)
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
        .args(capture_timeout_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some((path, attempt)) =
        crate::regression::archive::work_state::classic_telemetry_target()
    {
        command
            .arg("-TelemetryPath")
            .arg(windows_path(&path)?)
            .arg("-AttemptId")
            .arg(attempt)
            .arg("-ComparisonProfile")
            .arg(shrunk.file_stem().unwrap_or_default());
    }
    let mut child = command.spawn().context("start paired worksheet capture")?;
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
    let mut heights = Vec::new();
    let mut height_sheets = BTreeSet::new();
    let mut changed_rows = BTreeMap::new();
    let mut capture_columns = BTreeMap::new();
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
        if info["kind"] == "heights" {
            crate::regression::archive::work_state::event(
                "comparing",
                "classic_heights_comparing",
                "Comparing measured row heights from both Excel workbooks",
                false,
                None,
                json!({"sheet_index":info["sheet_index"]}),
            );
            let compared =
                compare_streamed_heights(&info, &left, &right, &selected, &mut height_sheets);
            match compared {
                Ok(changes) => {
                    let index = info["sheet_index"].as_u64().unwrap() as usize;
                    let rows: Vec<_> = changes.iter().map(|change| change.row).collect();
                    let response = serde_json::to_vec(&json!({"rows": rows}))?;
                    input.write_all(&[3])?;
                    input.write_all(&(response.len() as u32).to_le_bytes())?;
                    input.write_all(&response)?;
                    input.flush()?;
                    if !rows.is_empty() {
                        changed_rows.insert(index, rows);
                    }
                    heights.extend(changes);
                }
                Err(error) => {
                    consumer_error = Some(error);
                    input.write_all(&[0])?;
                    input.flush()?;
                    break;
                }
            }
            crate::regression::archive::work_state::event(
                "waiting_classic",
                "classic_heights_compared",
                "Row-height comparison finished; waiting for the next classic capture",
                true,
                Some("classic capture process"),
                json!({"sheet_index":info["sheet_index"]}),
            );
            continue;
        }
        crate::regression::archive::work_state::event(
            "comparing_images",
            "classic_images_comparing",
            "Comparing captured images from both Excel workbooks",
            false,
            None,
            json!({"sheet_index":info["sheet_index"],"sheet_name":info["sheet_name"],
                "range":info["range"],"tile_index":info["tile_index"]}),
        );
        let compared = (|| -> Result<()> {
            let capture_end = info["capture_right_column"].as_u64();
            let info: ImageTile = serde_json::from_value(info)?;
            ensure!(
                height_sheets.contains(&info.sheet_index),
                "image arrived before row-height comparison"
            );
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
                first >= 1 && last <= sheet.bottom_row + 1 && start_column == 1,
                "streamed range exceeds the worksheet plan"
            );
            validate_streamed_columns(sheet, capture_end, end_column, &mut capture_columns)?;
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
            if changed_rows.contains_key(&info.sheet_index) {
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
        } else {
            crate::regression::archive::work_state::event(
                "waiting_classic",
                "classic_tile_compared",
                "Captured tile compared; waiting for the next classic capture",
                true,
                Some("classic capture process"),
                json!({"compared_tiles":tiles.len()}),
            );
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
    crate::regression::launcher_journal::mark_exited(&mut child)?;
    let stderr = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
    transfer.publish(output)?;
    if let Ok(bytes) = fs::read(output.join("result.json")) {
        let result: OfficeResult = serde_json::from_slice(&bytes)?;
        if let Some(error) = result.error {
            if error.code == "TASKPANE_RESTARTED" {
                return Err(TaskpaneRestarted(error.message).into());
            }
            if let Some(comparison_error) = consumer_error.take() {
                return Err(comparison_error);
            }
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
    ensure!(
        height_sheets.len() == selected.len(),
        "paired capture omitted row heights"
    );
    for height_evidence in [false, true] {
        let sheets: Vec<_> = selected
            .iter()
            .copied()
            .filter(|sheet| changed_rows.contains_key(&sheet.sheet_index) == height_evidence)
            .collect();
        if sheets.is_empty() {
            continue;
        }
        let images = metadata
            .iter()
            .filter(|((index, _), _)| changed_rows.contains_key(index) == height_evidence)
            .map(|(key, value)| (*key, value.clone()))
            .collect();
        validate_tile_coverage(&images, &sheets, height_evidence.then_some(&changed_rows))?;
    }
    Ok(PairedVisualResult {
        tiles,
        differences,
        heights,
    })
}

fn validate_streamed_columns(
    sheet: &PairedSheetPlan,
    capture_end: Option<u64>,
    range_end: u32,
    received: &mut BTreeMap<usize, u32>,
) -> Result<()> {
    let end = capture_end.context("missing negotiated capture boundary")?;
    ensure!(
        end <= u64::from(sheet.capture_right_column) && end + 1 == u64::from(range_end),
        "streamed columns exceed the negotiated capture boundary"
    );
    for config in [&sheet.original_config, &sheet.shrunk_config] {
        let base = config["base_right_column"]
            .as_u64()
            .context("missing content boundary")?;
        ensure!(end >= base, "streamed columns omit worksheet content");
        if config["horizontal_overflow_required"] == true && base < 16383 {
            ensure!(end > base, "streamed columns omit the overflow probe");
        }
    }
    let end = end as u32;
    if let Some(previous) = received.get(&sheet.sheet_index) {
        ensure!(
            *previous == end,
            "streamed column boundary changed between tiles"
        );
    } else {
        received.insert(sheet.sheet_index, end);
    }
    Ok(())
}

fn compare_streamed_heights(
    metadata: &Value,
    original: &[u8],
    shrunk: &[u8],
    sheets: &[&PairedSheetPlan],
    received: &mut BTreeSet<usize>,
) -> Result<Vec<RowHeightDifference>> {
    let index = metadata["sheet_index"]
        .as_u64()
        .context("missing height sheet index")? as usize;
    let sheet = sheets
        .iter()
        .find(|sheet| sheet.sheet_index == index)
        .context("unexpected row-height worksheet")?;
    ensure!(received.insert(index), "duplicate row-height worksheet");
    let left: SheetHeights = serde_json::from_slice(original)?;
    let right: SheetHeights = serde_json::from_slice(shrunk)?;
    row_heights::compare_row_heights(
        &[left],
        &[right],
        &[SheetExtent {
            index,
            name: sheet.sheet_name.clone(),
            last_row: sheet.bottom_row + 1,
            last_checked_row: sheet.row_height_bottom_row + 1,
        }],
    )
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
            && u64::from(width) * u64::from(height) <= 64_000_000,
        "streamed PNG exceeds image bounds"
    );
    Ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.to_rgba8())
}

fn read_frame(reader: &mut impl Read) -> Result<Option<(Value, Vec<u8>, Vec<u8>)>> {
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
    fn smaller_tiles_retry_only_explicit_budget_or_runtime_failures() {
        assert!(retry_with_smaller_tiles(
            &TaskpaneRestarted("original".into()).into()
        ));
        assert!(retry_with_smaller_tiles(
            &ImageBudgetError("too large".into()).into()
        ));
        assert!(!retry_with_smaller_tiles(&anyhow::anyhow!(
            "Unexpected paired row heights"
        )));
    }

    #[test]
    fn streamed_columns_keep_content_overflow_and_a_stable_boundary() -> Result<()> {
        let sheet = PairedSheetPlan {
            sheet_index: 1,
            sheet_name: "Sheet1".into(),
            bottom_row: 9,
            row_height_bottom_row: 9,
            capture_right_column: 65,
            original_config: json!({"base_right_column":1,"horizontal_overflow_required":true}),
            shrunk_config: json!({"base_right_column":4,"horizontal_overflow_required":true}),
        };
        let mut received = BTreeMap::new();
        validate_streamed_columns(&sheet, Some(26), 27, &mut received)?;
        validate_streamed_columns(&sheet, Some(26), 27, &mut received)?;
        validate_streamed_columns(&sheet, Some(65), 66, &mut BTreeMap::new())?;
        let tile = ImageTile {
            sheet_index: 1,
            sheet_name: "Sheet1".into(),
            range: "A1:AA10".into(),
            tile_index: 0,
            image_path: PathBuf::new(),
        };
        validate_tile_coverage(&BTreeMap::from([((1, 0), tile)]), &[&sheet], None)?;
        assert!(validate_streamed_columns(&sheet, Some(27), 28, &mut received).is_err());
        for (end, range_end) in [
            (None, 27),
            (Some(66), 67),
            (Some(26), 28),
            (Some(3), 4),
            (Some(4), 5),
        ] {
            assert!(
                validate_streamed_columns(&sheet, end, range_end, &mut BTreeMap::new()).is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn height_frames_reject_missing_rows_and_duplicate_sheets() -> Result<()> {
        let sheet = PairedSheetPlan {
            sheet_index: 1,
            sheet_name: "Sheet1".into(),
            bottom_row: 9,
            row_height_bottom_row: 8,
            capture_right_column: 1,
            original_config: Value::Null,
            shrunk_config: Value::Null,
        };
        let metadata = json!({"kind":"heights","sheet_index":1});
        let original = serde_json::to_vec(&json!({"sheet_index":1,"sheet_name":"Sheet1",
            "runs":[{"start_row":1,"end_row":10,"height":15.0}]}))?;
        let shrunk = serde_json::to_vec(&json!({"sheet_index":1,"sheet_name":"Sheet1",
            "runs":[{"start_row":1,"end_row":9,"height":15.0},{"start_row":10,"end_row":10,"height":12.0}]}))?;
        let mut received = BTreeSet::new();
        assert!(
            compare_streamed_heights(&metadata, &original, &shrunk, &[&sheet], &mut received)?
                .is_empty()
        );
        assert!(
            compare_streamed_heights(&metadata, &original, &shrunk, &[&sheet], &mut received)
                .is_err()
        );
        let missing = serde_json::to_vec(&json!({"sheet_index":1,"sheet_name":"Sheet1",
            "runs":[{"start_row":1,"end_row":9,"height":15.0}]}))?;
        assert!(compare_streamed_heights(
            &metadata,
            &original,
            &missing,
            &[&sheet],
            &mut BTreeSet::new()
        )
        .is_err());
        Ok(())
    }

    #[test]
    #[ignore = "requires Windows Excel and the Office add-in runtime"]
    fn compares_heights_and_images_in_one_excel_session() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "es-paired-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir_all(&root)?;
        let original = root.join("original.xlsx");
        write_workbook(&original, false, 42)?;
        for (name, changed_height, value) in [
            ("same", false, 42),
            ("height", true, 42),
            ("value", false, 7),
        ] {
            let shrunk = root.join(format!("{name}.xlsx"));
            write_workbook(&shrunk, changed_height, value)?;
            let plan = plan_pair(&original, &shrunk)?;
            let result = compare_paired_visuals(&plan, &original, &shrunk, &root.join(name))?;
            assert_eq!(!result.heights.is_empty(), changed_height);
            assert_eq!(!result.differences.is_empty(), value != 42);
            if changed_height {
                assert!(result
                    .tiles
                    .iter()
                    .all(|tile| tile.original_image.is_file() && tile.shrunk_image.is_file()));
            }
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[ignore = "requires Windows Excel and explicit EXCEL_SHRINK_CAPTURE_{ORIGINAL,SHRUNK,OUTPUT} paths"]
    fn compares_external_workbooks() -> Result<()> {
        let path = |name: &str| -> Result<PathBuf> {
            Ok(std::env::var_os(name)
                .with_context(|| format!("set {name}"))?
                .into())
        };
        let original = path("EXCEL_SHRINK_CAPTURE_ORIGINAL")?;
        let shrunk = path("EXCEL_SHRINK_CAPTURE_SHRUNK")?;
        let output = path("EXCEL_SHRINK_CAPTURE_OUTPUT")?;
        let plan = plan_pair(&original, &shrunk)?;
        let result = compare_paired_visuals(&plan, &original, &shrunk, &output)?;
        println!(
            "Compared {} sheets: {} row-height differences, {} image differences",
            plan.sheets.len(),
            result.heights.len(),
            result.differences.len()
        );
        Ok(())
    }

    fn write_workbook(path: &Path, changed_height: bool, value: u32) -> Result<()> {
        let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B2"/><sheetData><row r="1" {}><c r="A1"><v>{value}</v></c><c r="B1"><v>8</v></c></row><row r="2"><c r="A2"><v>9</v></c></row></sheetData></worksheet>"#,
            if changed_height {
                "ht=\"25\" customHeight=\"1\""
            } else {
                ""
            }
        );
        for (name, xml) in [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ] {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(xml.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    }
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
