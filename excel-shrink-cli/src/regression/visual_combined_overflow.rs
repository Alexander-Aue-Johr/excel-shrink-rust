// Included by excel_automation.rs. Proportional XML column widths are intentional:
// this is an approximation, not Excel's font-dependent pixel-width conversion.
const OVERFLOW_MAX_COLUMN: u32 = 16_384;

struct OverflowColumnPair { original: Vec<f64>, shrunk: Vec<f64> }
impl OverflowColumnPair {
    fn new(left: &super::xlsx_value_compare::CaptureColumns, right: &super::xlsx_value_compare::CaptureColumns) -> anyhow::Result<Self> {
        fn expand(plan: &super::xlsx_value_compare::CaptureColumns) -> anyhow::Result<Vec<f64>> {
            anyhow::ensure!(plan.default_width.is_finite() && plan.default_width >= 0.0, "invalid default column width");
            let mut widths = vec![plan.default_width; OVERFLOW_MAX_COLUMN as usize + 1];
            widths[0] = 0.0;
            for span in &plan.spans {
                anyhow::ensure!(span.min > 0 && span.min <= span.max && span.max <= OVERFLOW_MAX_COLUMN, "invalid column span");
                let width = if span.hidden { 0.0 } else { span.width.unwrap_or(plan.default_width) };
                anyhow::ensure!(width.is_finite() && width >= 0.0, "invalid column width");
                widths[span.min as usize..=span.max as usize].fill(width);
            }
            Ok(widths)
        }
        Ok(Self { original: expand(left)?, shrunk: expand(right)? })
    }

    // Count visible columns, not worksheet indices. The union ensures a column
    // visible in only one workbook is still examined in the pair comparison.
    fn end_column(&self, base: u32, extra: u32) -> u32 {
        let mut found = 0;
        for col in base.saturating_add(1)..=OVERFLOW_MAX_COLUMN {
            if self.original[col as usize] > 0.0 || self.shrunk[col as usize] > 0.0 {
                found += 1;
                if found >= extra { return col; }
            }
        }
        if found == 0 { base } else { OVERFLOW_MAX_COLUMN }
    }
}

#[derive(Deserialize)]
struct OverflowColumnLimit {
    sheet_index: usize,
    original_columns: Option<super::xlsx_value_compare::CaptureColumns>,
    shrunk_columns: Option<super::xlsx_value_compare::CaptureColumns>,
}

#[derive(Deserialize)]
struct CombinedOverflowRequest {
    id: u64,
    sheet_index: usize,
    base_column: u32,
    end_column: u32,
    extra: u32,
    background_rgba: ScreenshotRgba,
    capture: ScreenshotCapture,
}

struct CombinedOverflowService {
    root: PathBuf,
    windows_root: String,
    columns: BTreeMap<usize, OverflowColumnPair>,
}

fn overflow_next_budget(extra: u32) -> u32 {
    if extra < 8 { 8 } else { extra.saturating_mul(2).min(OVERFLOW_MAX_COLUMN) }
}

fn overflow_column_edges(widths: &[f64], info: &ScreenshotCaptureInfo, image_width: u32) -> anyhow::Result<Vec<u32>> {
    let first = info.start_column;
    let last = first.checked_add(info.columns).and_then(|n| n.checked_sub(1)).unwrap_or(u32::MAX);
    anyhow::ensure!(first > 0 && first <= last && last <= OVERFLOW_MAX_COLUMN, "invalid image column range");
    anyhow::ensure!(image_width > 0, "empty screenshot width");
    let slice = &widths[first as usize..=last as usize];
    let total: f64 = slice.iter().sum();
    anyhow::ensure!(total > 0.0 && total.is_finite(), "screenshot has no visible XML column width");
    let mut sum = 0.0;
    let mut edges = Vec::with_capacity(slice.len() + 1);
    edges.push(0);
    for width in slice {
        sum += width;
        edges.push((sum / total * f64::from(image_width - 1)).round() as u32);
    }
    Ok(edges)
}

#[derive(Default)]
struct OverflowProbe { last_content: u32, edge_content: bool, thin_columns: u32, scanned_pixels: u64, ignored_rule_rows: u64 }

// Ignore only thin, nearly uniform horizontal rules spanning the entire
// captured interior. Inspect the full image width, not a single extra column:
// a glyph stroke or a local chart line must not be mistaken for a row border.
// This mask affects detection only; the screenshot itself stays unchanged.
fn overflow_horizontal_rules(image: &image::RgbImage, background: ScreenshotRgba) -> Vec<bool> {
    let mut candidates = vec![false; image.height() as usize];
    if image.width() < 16 { return candidates; }
    for y in 1..image.height().saturating_sub(1) {
        let reference = image.get_pixel(1, y).0;
        if reference[0].abs_diff(background.r) <= 14
            && reference[1].abs_diff(background.g) <= 14
            && reference[2].abs_diff(background.b) <= 14 { continue; }
        candidates[y as usize] = (2..image.width() - 1).all(|x| {
            let pixel = image.get_pixel(x, y).0;
            (0..3).all(|channel| pixel[channel].abs_diff(reference[channel]) <= 8)
        });
    }
    let mut mask = vec![false; candidates.len()];
    let mut start = 1;
    while start < candidates.len() {
        if !candidates[start] { start += 1; continue; }
        let mut end = start + 1;
        while end < candidates.len() && candidates[end] { end += 1; }
        // Thick solid areas remain content. Separate thin rules (including
        // double borders) may each be ignored.
        if end - start <= 4 { mask[start..end].fill(true); }
        start = end;
    }
    mask
}

fn probe_overflow_image(image: &image::RgbImage, info: &ScreenshotCaptureInfo, widths: &[f64], base: u32,
    last_visible: Option<u32>, background: ScreenshotRgba) -> anyhow::Result<OverflowProbe> {
    let edges = overflow_column_edges(widths, info, image.width())?;
    let last = info.start_column + info.columns - 1;
    let mut result = OverflowProbe { last_content: base, ..Default::default() };
    let horizontal_rules = overflow_horizontal_rules(image, background);
    result.ignored_rule_rows = horizontal_rules.iter().filter(|&&v| v).count() as u64;
    for col in (base + 1).max(info.start_column)..=last {
        let offset = (col - info.start_column) as usize;
        let left = edges[offset]; let right = edges[offset + 1];
        if right.saturating_sub(left) <= 1 { result.thin_columns += 1; continue; }
        // The left border of the first probe column can belong to the base range.
        let first_x = if widths[(base + 1) as usize..col as usize].iter().all(|w| *w == 0.0) {
            left.saturating_add(1)
        } else { left };
        let mut content = false;
        'pixels: for y in 1..image.height().saturating_sub(1) {
            if horizontal_rules[y as usize] { continue; }
            for x in first_x.max(1)..right.min(image.width().saturating_sub(1)) {
                let p = image.get_pixel(x, y).0;
                result.scanned_pixels += 1;
                if p[0].abs_diff(background.r) > 14 || p[1].abs_diff(background.g) > 14 || p[2].abs_diff(background.b) > 14 {
                    content = true; break 'pixels;
                }
            }
        }
        if content {
            result.last_content = result.last_content.max(col);
            if Some(col) == last_visible { result.edge_content = true; }
        }
    }
    Ok(result)
}

fn overflow_address(info: &ScreenshotCaptureInfo) -> String {
    fn cell(mut column: u32, row: u32) -> String {
        let mut name = String::new();
        while column > 0 { column -= 1; name.insert(0, (b'A' + (column % 26) as u8) as char); column /= 26; }
        format!("{name}{row}")
    }
    format!("{}:{}", cell(info.start_column, info.start_row),
        cell(info.start_column + info.columns - 1, info.start_row + info.rows - 1))
}

fn overflow_trim_info(info: &mut ScreenshotCaptureInfo, keep: u32) {
    let last = info.start_column + info.columns - 1;
    info.columns = last.min(keep) - info.start_column + 1;
    info.address = overflow_address(info);
    info.strategy = "combined_overflow_proportional".into();
}

impl CombinedOverflowService {
    fn new(root: &Path, limits: &Path) -> anyhow::Result<Self> {
        let plans: Vec<OverflowColumnLimit> = read_json_file(limits)?;
        let mut columns = BTreeMap::new();
        for plan in plans {
            if let (Some(a), Some(b)) = (plan.original_columns, plan.shrunk_columns) {
                columns.insert(plan.sheet_index, OverflowColumnPair::new(&a, &b)?);
            }
        }
        fs::create_dir_all(root.join("images/cropped"))?;
        Ok(Self { root: root.to_owned(), windows_root: power_shell_path(root)?, columns })
    }

    fn poll(&mut self) -> anyhow::Result<bool> {
        let path = self.root.join("overflow-request.json");
        if !path.is_file() { return Ok(false); }
        // PowerShell atomically publishes complete requests, never partial JSON.
        let request: CombinedOverflowRequest = read_json_file(&path)?;
        fs::remove_file(&path)?;
        let id = request.id;
        let start = Instant::now();
        let result = self.analyze(request);
        let response = match result {
            Ok(mut response) => { response["id"] = serde_json::json!(id); response },
            Err(error) => serde_json::json!({"id": id, "error": format!("{error:#}")}),
        };
        atomic_cache_write(&self.root.join(format!("overflow-response-{id}.json")), &serde_json::to_vec(&response)?)?;
        tracing::info!("[overflow] request={id} rust_analysis_and_crop_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
        Ok(true)
    }

    fn analyze(&self, request: CombinedOverflowRequest) -> anyhow::Result<serde_json::Value> {
        let _phase = visual_diagnostics::phase("screenshot.combined_overflow", format!("sheet={} request={}", request.sheet_index, request.id));
        anyhow::ensure!(request.base_column > 0 && request.base_column <= request.end_column && request.end_column <= OVERFLOW_MAX_COLUMN,
            "invalid overflow request bounds");
        anyhow::ensure!(request.capture.sheet_index == request.sheet_index && request.extra > 0, "invalid overflow request identity");
        let columns = self.columns.get(&request.sheet_index).ok_or_else(|| anyhow::anyhow!("missing Rust column plan"))?;
        let info = request.capture.capture_info.as_ref().ok_or_else(|| anyhow::anyhow!("capture info missing"))?;
        anyhow::ensure!(info.start_column == 1 && info.columns == request.end_column, "capture extent differs from request");
        let mut keep = request.base_column;
        let mut edge_content = false;
        let mut thin = 0;
        let mut pixels = 0;
        let mut ignored_rule_rows = 0;
        for (widths, original) in [(&columns.original, true), (&columns.shrunk, false)] {
            let last_visible = (request.base_column + 1..=request.end_column).rev().find(|&c| widths[c as usize] > 0.0);
            let more_visible = (request.end_column + 1..=OVERFLOW_MAX_COLUMN).any(|c| widths[c as usize] > 0.0);
            let mut inspect = |path: &Option<PathBuf>, tile: &ScreenshotCaptureInfo| -> anyhow::Result<()> {
                if tile.start_column + tile.columns - 1 <= request.base_column || last_visible.is_none() { return Ok(()); }
                let path = path.as_ref().ok_or_else(|| anyhow::anyhow!("missing capture image"))?;
                let image = image::open(normalize_automation_path(path))?.into_rgb8();
                let probe = probe_overflow_image(&image, tile, widths, request.base_column, last_visible, request.background_rgba)?;
                keep = keep.max(probe.last_content);
                edge_content |= probe.edge_content && more_visible;
                thin += probe.thin_columns; pixels += probe.scanned_pixels;
                ignored_rule_rows += probe.ignored_rule_rows;
                Ok(())
            };
            if request.capture.tiles.is_empty() {
                inspect(if original { &request.capture.original_image } else { &request.capture.shrunk_image }, info)?;
            } else {
                for tile in &request.capture.tiles {
                    inspect(if original { &tile.original_image } else { &tile.shrunk_image }, &tile.capture_info)?;
                }
            }
        }
        let next_extra = overflow_next_budget(request.extra);
        let next_column = columns.end_column(request.base_column, next_extra);
        let done = !edge_content || next_column <= request.end_column;
        tracing::info!("[overflow] sheet={} base={} captured={} keep={} done={done} thin_columns={thin} scanned_pixels={pixels} ignored_rule_rows={ignored_rule_rows} next={next_column}",
            request.sheet_index, request.base_column, request.end_column, keep);
        if !done {
            return Ok(serde_json::json!({"done": false, "next_column": next_column, "next_extra": next_extra}));
        }
        let mut capture = request.capture;
        let mut counter = 0usize;
        if capture.tiles.is_empty() {
            let original_info = capture.capture_info.as_ref().unwrap();
            self.crop_image(&mut capture.original_image, original_info, &columns.original, keep, request.id, &mut counter)?;
            self.crop_image(&mut capture.shrunk_image, original_info, &columns.shrunk, keep, request.id, &mut counter)?;
        } else {
            capture.tiles.retain(|t| t.capture_info.start_column <= keep);
            for (i, tile) in capture.tiles.iter_mut().enumerate() {
                let (w, h) = self.crop_image(&mut tile.original_image, &tile.capture_info, &columns.original, keep, request.id, &mut counter)?;
                tile.original_width = Some(w); tile.original_height = Some(h);
                let (w, h) = self.crop_image(&mut tile.shrunk_image, &tile.capture_info, &columns.shrunk, keep, request.id, &mut counter)?;
                tile.shrunk_width = Some(w); tile.shrunk_height = Some(h);
                overflow_trim_info(&mut tile.capture_info, keep);
                tile.capture_range = tile.capture_info.address.clone(); tile.tile_index = i + 1;
            }
        }
        let info = capture.capture_info.as_mut().unwrap();
        overflow_trim_info(info, keep);
        capture.capture_range = Some(info.address.clone());
        capture.capture_strategy = Some(info.strategy.clone());
        Ok(serde_json::json!({"done": true, "keep_column": keep, "capture": capture}))
    }

    fn crop_image(&self, path: &mut Option<PathBuf>, info: &ScreenshotCaptureInfo, widths: &[f64], keep: u32,
        id: u64, counter: &mut usize) -> anyhow::Result<(u32, u32)> {
        let input = path.as_ref().ok_or_else(|| anyhow::anyhow!("missing crop input"))?;
        let input = normalize_automation_path(input);
        if info.start_column + info.columns - 1 <= keep { return Ok(image::image_dimensions(input)?); }
        let image = image::open(input)?;
        let edges = overflow_column_edges(widths, info, image.width())?;
        let end = (keep - info.start_column + 1) as usize;
        let width = edges[end].saturating_add(1).min(image.width()).max(1);
        let height = image.height();
        if width == image.width() { return Ok((width, height)); }
        *counter += 1;
        let name = format!("{id}-{}.png", *counter);
        image.crop_imm(0, 0, width, height).save(self.root.join("images/cropped").join(&name))?;
        *path = Some(PathBuf::from(format!("{}\\images\\cropped\\{name}", self.windows_root)));
        Ok((width, height))
    }
}

#[cfg(test)]
mod combined_overflow_tests {
    use super::*;
    use super::super::xlsx_value_compare::CaptureColumns;

    fn plan(json: serde_json::Value) -> CaptureColumns { serde_json::from_value(json).unwrap() }
    fn info(start: u32, columns: u32) -> ScreenshotCaptureInfo {
        ScreenshotCaptureInfo { strategy: "test".into(), address: "test".into(), start_row: 1,
            start_column: start, rows: 20, columns, used_range_error: None }
    }
    #[test]
    fn hidden_columns_do_not_consume_budget_and_xfd_is_inclusive() {
        let p = plan(serde_json::json!({"default_width":10,"spans":[{"min":5,"max":104,"hidden":true,"width":10}]}));
        let pair = OverflowColumnPair::new(&p, &p).unwrap();
        assert_eq!(pair.end_column(3,1),4);
        assert_eq!(pair.end_column(3,8),111); // 8 visible + 100 hidden
        assert_eq!(pair.end_column(16382,8),16384);
        assert_eq!(pair.end_column(16384,1),16384);
        let p = plan(serde_json::json!({"default_width":10,"spans":[{"min":4,"max":16384,"hidden":true,"width":10}]}));
        let pair = OverflowColumnPair::new(&p, &p).unwrap();
        assert_eq!(pair.end_column(3,1),3);
        assert_eq!(pair.end_column(3,8),3);
        assert_eq!([1,8,16,32].map(overflow_next_budget),[8,16,32,64]);
    }
    #[test]
    fn pair_counts_column_visible_on_either_side_and_width_zero_as_hidden() {
        let a=plan(serde_json::json!({"default_width":10,"spans":[{"min":4,"max":104,"hidden":true,"width":10}]}));
        let b=plan(serde_json::json!({"default_width":10,"spans":[{"min":4,"max":4,"hidden":false,"width":0}]}));
        let pair=OverflowColumnPair::new(&a,&b).unwrap();
        assert_eq!(pair.end_column(3,1),5);
        assert_eq!(pair.end_column(3,8),12);
    }
    #[test]
    fn proportional_pixels_detect_content_and_skip_one_pixel_columns() {
        let mut widths=vec![10.0;16385];widths[2]=20.0;
        let mut image=image::RgbImage::from_pixel(401,20,image::Rgb([255,255,255]));
        let bg=ScreenshotRgba{r:255,g:255,b:255,a:255};
        assert_eq!(overflow_column_edges(&widths,&info(1,3),401).unwrap(),vec![0,100,300,400]);
        assert!(!probe_overflow_image(&image,&info(1,3),&widths,2,Some(3),bg).unwrap().edge_content);
        image.put_pixel(320,7,image::Rgb([0,0,0]));
        let result=probe_overflow_image(&image,&info(1,3),&widths,2,Some(3),bg).unwrap();
        assert!(result.edge_content);assert_eq!(result.last_content,3);
        widths[3]=0.01;
        let result=probe_overflow_image(&image,&info(1,3),&widths,2,Some(3),bg).unwrap();
        assert!(!result.edge_content);assert_eq!(result.thin_columns,1);
    }

    #[test]
    fn combined_overflow_ignores_row_rules_but_preserves_text_and_chart_content() {
        let widths = vec![10.0; 16385];
        let bg = ScreenshotRgba { r:255, g:255, b:255, a:255 };
        let white = image::Rgb([255,255,255]);
        let black = image::Rgb([0,0,0]);
        let mut image = image::RgbImage::from_pixel(301,40,white);
        // Thin, double and antialiased full-width row borders.
        for (y, color) in [(8,black),(9,image::Rgb([100,100,100])),(20,black),(23,black)] {
            for x in 0..301 { image.put_pixel(x,y,color); }
        }
        let probe = |image: &image::RgbImage| probe_overflow_image(image,&info(1,3),&widths,2,Some(3),bg).unwrap();
        let result = probe(&image);
        assert!(!result.edge_content);
        assert_eq!(result.last_content,2);
        assert_eq!(result.ignored_rule_rows,4);
        // Text crossing a rule still has visible pixels above/below it.
        for y in 5..14 { image.put_pixel(230,y,black); }
        assert!(probe(&image).edge_content);
        // A local horizontal glyph/chart stroke must remain content.
        let mut local = image::RgbImage::from_pixel(301,40,white);
        for x in 215..280 { local.put_pixel(x,12,black); }
        assert!(probe(&local).edge_content);
        // Thick uniform chart/fill areas must remain content as well.
        let mut thick = image::RgbImage::from_pixel(301,40,white);
        for y in 10..20 { for x in 0..301 { thick.put_pixel(x,y,black); } }
        assert!(probe(&thick).edge_content);
        // Color variation along a chart line must not be suppressed.
        image.put_pixel(240,20,image::Rgb([255,0,0]));
        assert!(!overflow_horizontal_rules(&image,bg)[20]);
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let stamp=SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
            let path=std::env::temp_dir().join(format!("combined-overflow-{}-{stamp}",std::process::id()));
            fs::create_dir_all(path.join("images/cropped")).unwrap();Self(path)
        }
    }
    impl Drop for Fixture {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    fn request(root:&Path,end:u32,extra:u32,tiles:bool) -> CombinedOverflowRequest {
        let left=root.join("original.png");let right=root.join("shrunk.png");
        let capture_info=info(1,end);
        let capture=ScreenshotCapture {sheet_index:1,sheet_name:"fixture".into(),capture_range:Some(overflow_address(&capture_info)),
            capture_strategy:Some("test".into()),used_range_error:None,screenshot_error:None,
            original_image:if tiles{None}else{Some(left.clone())},shrunk_image:if tiles{None}else{Some(right.clone())},
            original_background_rgba:None,shrunk_background_rgba:None,capture_info:Some(capture_info.clone()),
            tiles:if tiles{vec![ScreenshotCaptureTile{tile_index:1,capture_range:overflow_address(&capture_info),capture_info,
                original_image:Some(left),shrunk_image:Some(right),original_background_rgba:None,shrunk_background_rgba:None,
                original_width:None,original_height:None,shrunk_width:None,shrunk_height:None}]}else{vec![]}};
        CombinedOverflowRequest{id:1,sheet_index:1,base_column:2,end_column:end,extra,
            background_rgba:ScreenshotRgba{r:255,g:255,b:255,a:255},capture}
    }
    #[test]
    fn saved_images_are_reused_and_both_sides_crop_to_shared_content_bound() {
        for tiles in [false,true] {
            let f=Fixture::new();
            let p=plan(serde_json::json!({"default_width":10,"spans":[]}));
            let mut columns=BTreeMap::new();columns.insert(1,OverflowColumnPair::new(&p,&p).unwrap());
            let service=CombinedOverflowService{root:f.0.clone(),windows_root:"C:\\fixture".into(),columns};
            let left=image::RgbImage::from_pixel(301,20,image::Rgb([255,255,255]));
            let mut right=left.clone();right.put_pixel(230,8,image::Rgb([0,0,0]));
            left.save(f.0.join("original.png")).unwrap();right.save(f.0.join("shrunk.png")).unwrap();
            let first=service.analyze(request(&f.0,3,1,tiles)).unwrap();
            assert_eq!(first["done"],false);assert_eq!(first["next_column"],10);
            let left=image::RgbImage::from_pixel(1001,20,image::Rgb([255,255,255]));
            let mut right=left.clone();right.put_pixel(630,8,image::Rgb([0,0,0])); // seventh column
            left.save(f.0.join("original.png")).unwrap();right.save(f.0.join("shrunk.png")).unwrap();
            let last=service.analyze(request(&f.0,10,8,tiles)).unwrap();
            assert_eq!(last["done"],true);assert_eq!(last["keep_column"],7);
            assert_eq!(last["capture"]["capture_info"]["columns"],7);
            assert_eq!(image::image_dimensions(f.0.join("images/cropped/1-1.png")).unwrap(),(701,20));
            assert_eq!(image::image_dimensions(f.0.join("images/cropped/1-2.png")).unwrap(),(701,20));
            assert_eq!(image::image_dimensions(f.0.join("original.png")).unwrap(),(1001,20));
        }
    }

    #[test]
    #[ignore = "requires Windows PowerShell through Windows or WSL; no Excel is started"]
    fn combined_overflow_power_shell_bridge() -> anyhow::Result<()> {
        let f=Fixture::new();
        let p=plan(serde_json::json!({"default_width":10,"spans":[]}));
        let limits=f.0.join("limits.json");
        fs::write(&limits,serde_json::to_vec(&serde_json::json!([{"sheet_index":1,"original_columns":p,"shrunk_columns":p}]))?)?;
        let image=image::RgbImage::from_pixel(301,20,image::Rgb([255,255,255]));
        image.save(f.0.join("original.png"))?;image.save(f.0.join("shrunk.png"))?;
        fs::write(f.0.join("capture.json"),serde_json::to_vec(&request(&f.0,3,1,false).capture)?)?;
        fs::write(f.0.join("excel_combined_overflow.ps1"),include_str!("excel_combined_overflow.ps1"))?;
        let script=f.0.join("bridge.ps1");
        fs::write(&script,r#"param([string]$RangeLimitPath,[string]$ProgressPath)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
function Write-ProgressLine($message){Add-Content -LiteralPath $ProgressPath -Value $message}
Set-Content -LiteralPath $ProgressPath -Value 'bridge start'
. (Join-Path $PSScriptRoot 'excel_combined_overflow.ps1')
$script:OverflowBackground=[Drawing.Color]::White.ToArgb()
$capture=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'capture.json') -Raw | ConvertFrom-Json
$result=Invoke-CombinedOverflowAnalysis $capture 1 2 3 1
if(-not $result.done -or $result.keep_column -ne 2){throw 'Unexpected Rust reply'}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'bridge-result.json'),($result | ConvertTo-Json -Depth 16),[Text.UTF8Encoding]::new($false))
"#)?;
        let progress=f.0.join("progress.log");
        let status=run_power_shell_script_with_status_and_stall_timeout(&script,&[
            ("RangeLimitPath",power_shell_path(&limits)?),("ProgressPath",power_shell_path(&progress)?)],
            &progress,Duration::from_secs(30))?;
        assert!(matches!(status,PowerShellScriptOutcome::Completed(s) if s.success()));
        let result:serde_json::Value=read_json_file(&f.0.join("bridge-result.json"))?;
        assert_eq!(result["capture"]["capture_info"]["columns"],2);
        assert_eq!(image::image_dimensions(f.0.join("images/cropped/1-1.png"))?,(201,20));
        Ok(())
    }
}
