//! Pure Rust tests; no Excel or PowerShell process is started here.
use super::*;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> anyhow::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("es-visual-test-{}-{stamp}-{serial}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn fixture_parts() -> Vec<(String, Vec<u8>)> {
    [
        ("xl/workbook.xml", r#"<workbook><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="3" r:id="rId3"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#),
        ("xl/worksheets/sheet2.xml", r#"<worksheet><sheetData/></worksheet>"#),
        ("xl/chartsheets/sheet1.xml", "<chartsheet/>"),
        ("xl/styles.xml", "<styleSheet/>"),
        ("xl/sharedStrings.xml", "<sst/>"),
        ("xl/charts/chart1.xml", "<chartSpace/>"),
        ("xl/drawings/drawing1.xml", "<wsDr/>"),
        ("xl/media/image1.png", "fixture-image-data"),
    ].into_iter().map(|(n, b)| (n.into(), b.as_bytes().to_vec())).collect()
}
fn write_zip(path: &Path, parts: &[(String, Vec<u8>)], compressed: bool) -> anyhow::Result<()> {
    let mut zip = ZipWriter::new(File::create(path)?);
    let method = if compressed { CompressionMethod::Deflated } else { CompressionMethod::Stored };
    let options = FileOptions::default().compression_method(method).last_modified_time(
        zip::DateTime::from_date_and_time(if compressed { 2025 } else { 2024 }, 1, 1, 0, 0, 0).unwrap());
    for (name, bytes) in parts { zip.start_file(name, options)?; zip.write_all(bytes)?; }
    zip.finish()?;
    Ok(())
}
fn cache_and_png(dir: &Path) -> anyhow::Result<(WorkbookScreenshotCache, PathBuf)> {
    let workbook = dir.join("input.xlsx");
    write_zip(&workbook, &fixture_parts(), true)?;
    let cache = WorkbookScreenshotCache::new(&workbook, &dir.join("cache"), "requested-range")?;
    let image = dir.join("raw.png");
    image::RgbaImage::from_pixel(20, 10, image::Rgba([240, 30, 90, 255])).save(&image)?;
    Ok((cache, image))
}
fn partial_manifest(dir: &Path, image: &Path) -> anyhow::Result<PathBuf> {
    let path = dir.join("captures.json");
    let data = serde_json::json!([{
        "sheet_index":1,"sheet_name":"Data","original_image":image,
        "capture_range":"A1:D6","capture_strategy":"test",
        "capture_info":{"strategy":"test","address":"A1:D6","start_row":1,"start_column":1,"rows":6,"columns":4},
        "tiles":[]
    }]);
    fs::write(&path, serde_json::to_vec(&data)?)?;
    Ok(path)
}
fn bounds(rows: u32, columns: u32, source: &str) -> WorksheetCaptureRange {
    WorksheetCaptureRange { sheet_index:1, sheet_file:"xl/worksheets/sheet1.xml".into(),
        max_cell:"D6".into(),max_row:rows,max_column:columns,
        max_row_source:Some(source.into()),max_column_source:Some(source.into()) }
}

#[test]
fn package_identity_ignores_zip_order_compression_and_timestamps() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?;
    let a = dir.0.join("a.xlsx"); let b = dir.0.join("renamed.xlsx");
    let mut parts = fixture_parts();
    write_zip(&a, &parts, false)?; parts.reverse(); write_zip(&b, &parts, true)?;
    assert_ne!(file_content_hash(&a)?, file_content_hash(&b)?);
    assert_eq!(workbook_content_hash(&a)?, workbook_content_hash(&b)?);
    let ca = WorkbookScreenshotCache::new(&a, &dir.0.join("cache"), "range")?;
    let cb = WorkbookScreenshotCache::new(&b, &dir.0.join("cache"), "range")?;
    assert_eq!(ca.cache_dir, cb.cache_dir);
    Ok(())
}
#[test]
fn rendering_dependencies_invalidate_even_when_sheet_xml_is_unchanged() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?;
    let a = dir.0.join("a.xlsx"); let b = dir.0.join("b.xlsx");
    let parts = fixture_parts(); write_zip(&a, &parts, true)?;
    let baseline = workbook_content_hash(&a)?;
    for dependency in ["xl/workbook.xml", "xl/styles.xml", "xl/sharedStrings.xml", "xl/charts/chart1.xml", "xl/drawings/drawing1.xml", "xl/media/image1.png"] {
        let mut changed = parts.clone();
        changed.iter_mut().find(|(name, _)| name == dependency).unwrap().1.push(b' ');
        write_zip(&b, &changed, true)?;
        assert_ne!(baseline, workbook_content_hash(&b)?, "{dependency}");
    }
    Ok(())
}
#[test]
fn bounds_diagnostics_do_not_change_keys_but_geometry_does() -> anyhow::Result<()> {
    assert_eq!(capture_range_hash(&[bounds(6, 4, "A6")])?, capture_range_hash(&[bounds(6, 4, "B6")])?);
    assert_ne!(capture_range_hash(&[bounds(6, 4, "A6")])?, capture_range_hash(&[bounds(6, 5, "A6")])?);
    Ok(())
}
#[test]
fn chart_sheets_follow_worksheet_indices() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let path = dir.0.join("charts.xlsx");
    write_zip(&path, &fixture_parts(), true)?;
    assert_eq!(workbook_visual_sheet_order(&path)?, vec![
        "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml", "xl/chartsheets/sheet1.xml"]);
    Ok(())
}
#[test]
fn publishing_is_immutable_and_new_pixels_never_reuse_old_target() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let (cache, png) = cache_and_png(&dir.0)?;
    let identity = &cache.sheet_identities[&1];
    let first = materialize_shared_screenshot_image(&cache, identity, 1, "Data", true, None, &png)?;
    let bytes = fs::read(&first.path)?;
    let second = materialize_shared_screenshot_image(&cache, identity, 1, "Data", false, None, &first.path)?;
    assert_eq!(first.path, second.path);
    assert_eq!(first.image_hash, second.image_hash);
    assert_eq!(bytes, fs::read(&first.path)?);
    image::RgbaImage::from_pixel(20, 10, image::Rgba([10, 220, 25, 255])).save(&png)?;
    let changed = materialize_shared_screenshot_image(&cache, identity, 1, "Data", true, None, &png)?;
    assert_ne!(changed.path, first.path);
    assert_eq!(bytes, fs::read(first.path)?);
    Ok(())
}
#[test]
fn partial_results_survive_and_damaged_images_become_misses() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let (cache, png) = cache_and_png(&dir.0)?;
    let manifest = partial_manifest(&dir.0, &png)?;
    update_workbook_screenshot_cache(&manifest, &cache, true)?;
    let valid = cache.valid_current_sheet_captures()?;
    assert_eq!(valid.len(), 1);
    assert!(!cache.is_complete()?); assert!(!cache.marker_path.exists());
    let published = valid[&1].image.as_ref().unwrap();
    assert_ne!(published, &png);
    let rewritten: Vec<ScreenshotCapture> = read_json_file(&manifest)?;
    assert_eq!(rewritten[0].original_image.as_ref(), Some(published));
    fs::write(published, b"corrupted")?;
    assert!(cache.valid_current_sheet_captures()?.is_empty());
    Ok(())
}
#[test]
fn same_package_with_different_requested_ranges_has_separate_manifest() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let (first, _) = cache_and_png(&dir.0)?;
    let second = WorkbookScreenshotCache::new(&dir.0.join("input.xlsx"), &dir.0.join("cache"), "other-range")?;
    assert_ne!(first.manifest_path, second.manifest_path);
    Ok(())
}
#[test]
fn geometry_mismatch_cannot_take_full_pair_cache_fast_path() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let (cache, png) = cache_and_png(&dir.0)?;
    let manifest = partial_manifest(&dir.0, &png)?;
    update_workbook_screenshot_cache(&manifest, &cache, true)?;
    let a = cache.valid_current_sheet_captures()?.remove(&1).unwrap();
    let mut b = a.clone();
    assert!(capture_geometry_matches(&a, &b));
    b.capture_info.as_mut().unwrap().columns += 1;
    assert!(!capture_geometry_matches(&a, &b));
    Ok(())
}
#[cfg(not(windows))]
#[test]
fn unc_conversion_maps_only_the_current_distribution() {
    assert_eq!(wsl_unc_path(r"\\wsl.localhost\Ubuntu-26.04\home\aue-johr\a.png", Some("Ubuntu-26.04")), Some("/home/aue-johr/a.png".into()));
    assert_eq!(wsl_unc_path(r"\\wsl$\Ubuntu\tmp\a.png", Some("Ubuntu")), Some("/tmp/a.png".into()));
    assert_eq!(wsl_unc_path(r"\\?\UNC\wsl.localhost\Ubuntu\tmp\a.png", Some("Ubuntu")), Some("/tmp/a.png".into()));
    assert!(wsl_unc_path(r"\\wsl.localhost\Other\tmp\a.png", Some("Ubuntu")).is_none());
    assert!(wsl_unc_path(r"\\wsl.localhost\Ubuntu\tmp\a.png", None).is_none());
    assert!(wsl_unc_path(r"\\server\share\a.png", Some("Ubuntu")).is_none());
}
#[test]
fn progress_tail_handles_partial_lines_and_truncation() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let path = dir.0.join("progress.log");
    let mut tail = ExcelProgressTail::new("test child (no Excel)");
    assert!(!tail.poll(&path)?);
    fs::write(&path, b"sheet start")?; assert!(tail.poll(&path)?);
    assert_eq!(tail.pending, b"sheet start");
    fs::OpenOptions::new().append(true).open(&path)?.write_all(b" index=1\n")?;
    assert!(tail.poll(&path)?); assert!(tail.pending.is_empty());
    assert!(!tail.poll(&path)?);
    fs::write(&path, b"new\n")?; assert!(tail.poll(&path)?);
    assert_eq!(tail.offset, 4); assert!(tail.pending.is_empty());
    Ok(())
}
#[test]
fn missing_both_images_is_a_failed_capture_not_a_visual_pass() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let path = dir.0.join("captures.json");
    fs::write(&path, br#"[{"sheet_index":1,"sheet_name":"Data","tiles":[]}]"#)?;
    let summary = compare_captured_screenshots(&path, &dir.0.join("differences.json"), None)?;
    assert_eq!(summary.screenshot_error_differences, 1);
    Ok(())
}
#[test]
fn combined_bounds_include_original_and_shrunk_extents() {
    let report = super::super::xlsx_value_compare::WorkbookComparisonReport {
        compared_sheets:1, difference_count:0, differences:vec![],
        left_capture_ranges:vec![bounds(20, 4, "A20")], right_capture_ranges:vec![bounds(6, 8, "H6")],
    };
    let union = super::super::xlsx_value_compare::combined_capture_ranges(&report);
    assert_eq!(union[0].max_row, 20); assert_eq!(union[0].max_column, 8); assert_eq!(union[0].max_cell, "H20");
}
#[test]
fn tile_manifest_with_a_gap_or_overlap_is_not_complete() -> anyhow::Result<()> {
    let dir = TestDirectory::new()?; let (cache, png) = cache_and_png(&dir.0)?;
    let manifest = partial_manifest(&dir.0, &png)?;
    update_workbook_screenshot_cache(&manifest, &cache, true)?;
    let mut capture = cache.valid_current_sheet_captures()?.remove(&1).unwrap();
    let mut info = capture.capture_info.clone().unwrap();
    info.rows = 3;
    let top = WorkbookScreenshotTile { tile_index:1, capture_range:"A1:D3".into(),
        capture_info:info.clone(), image:capture.image.clone(), image_hash:capture.image_hash.clone(),
        background_rgba:None, width:None, height:None };
    let mut bottom = top.clone(); bottom.tile_index=2; bottom.capture_range="A4:D6".into();
    bottom.capture_info.start_row=4;
    capture.tiles = vec![top.clone(),bottom.clone()];
    assert!(capture_tile_coverage_is_complete(&capture));
    capture.tiles.pop();
    assert!(!capture_tile_coverage_is_complete(&capture));
    // Equal summed area, but overlapping tiles leave a gap elsewhere.
    bottom.capture_info.start_row=3;
    capture.tiles=vec![top,bottom];
    assert!(!capture_tile_coverage_is_complete(&capture));
    Ok(())
}
