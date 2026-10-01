//! Regression tests for guarded stored-state reuse. No Excel/PowerShell is started.
use super::*;

struct FixtureDirectory(PathBuf);
impl FixtureDirectory {
    fn new() -> anyhow::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("es-render-input-test-{}-{stamp}-{n}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for FixtureDirectory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn parts() -> BTreeMap<String, Vec<u8>> {
    [
        ("[Content_Types].xml", "<Types/>"),
        ("_rels/.rels", r#"<Relationships><Relationship Id="w" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="c" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#),
        ("docProps/core.xml", "<core>2026-01-01</core>"),
        ("xl/workbook.xml", r#"<workbook><sheets><sheet name="Data" sheetId="1" r:id="r1"/><sheet name="Other" sheetId="2" r:id="r2"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", r#"<Relationships><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="r2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="st" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="ss" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="th" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData><drawing r:id="d"/></worksheet>"#),
        ("xl/worksheets/sheet2.xml", "<worksheet><sheetData/></worksheet>"),
        ("xl/styles.xml", "<styleSheet/>"), ("xl/sharedStrings.xml", "<sst><si><t>Hello</t></si></sst>"),
        ("xl/theme/theme1.xml", "<theme/>"),
        ("xl/worksheets/_rels/sheet1.xml.rels", r#"<Relationships><Relationship Id="d" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#),
        ("xl/drawings/drawing1.xml", "<wsDr/>"),
        ("xl/drawings/_rels/drawing1.xml.rels", r#"<Relationships><Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="p" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#),
        ("xl/charts/chart1.xml", "<chartSpace><chart/></chartSpace>"),
        ("xl/media/image1.png", "image fixture"),
    ].into_iter().map(|(name, data)| (name.to_owned(), data.as_bytes().to_vec())).collect()
}
fn archive(path: &Path, parts: &BTreeMap<String, Vec<u8>>, reverse: bool) -> anyhow::Result<()> {
    let mut writer = ZipWriter::new(File::create(path)?);
    let options = FileOptions::default().compression_method(if reverse { CompressionMethod::Deflated } else { CompressionMethod::Stored })
        .last_modified_time(zip::DateTime::from_date_and_time(if reverse {2025} else {2024}, 1, 1, 0, 0, 0).unwrap());
    let mut items: Vec<_> = parts.iter().collect();
    if reverse { items.reverse(); }
    for (name, data) in items { writer.start_file(name.as_str(), options)?; writer.write_all(data)?; }
    writer.finish()?; Ok(())
}
fn changed(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> BTreeMap<String, Vec<u8>> {
    let mut changed = parts.clone(); changed.get_mut(name).unwrap().extend_from_slice(b" "); changed
}
fn bounds(index: usize, rows: u32, cols: u32) -> WorksheetCaptureRange {
    WorksheetCaptureRange { sheet_index:index, sheet_file:format!("xl/worksheets/sheet{index}.xml"), max_cell:"D6".into(),
        max_row:rows, max_column:cols, max_row_source:None, max_column_source:None }
}

#[test]
fn visual_package_reordering_has_identical_render_keys() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?; let a = dir.0.join("a.xlsx"); let b = dir.0.join("b.xlsx");
    archive(&a, &parts(), false)?; archive(&b, &parts(), true)?;
    assert_ne!(file_content_hash(&a)?, file_content_hash(&b)?);
    assert_eq!(workbook_content_hash(&a)?, workbook_content_hash(&b)?);
    let left = workbook_render_identities(&a)?; let right = workbook_render_identities(&b)?;
    for index in [1, 2] { assert_eq!(left[&index].sheet_render_hash, right[&index].sheet_render_hash); }
    Ok(())
}
#[test]
fn visual_simple_unchanged_sheet_survives_other_sheet_changes_but_not_global_changes() -> anyhow::Result<()> {
    let dir=FixtureDirectory::new()?;let a=dir.0.join("a.xlsx");let b=dir.0.join("b.xlsx");let base=plain_parts();
    archive(&a,&base,false)?;archive(&b,&changed(&base,"xl/worksheets/sheet2.xml"),true)?;
    let left=workbook_render_identities(&a)?;let right=workbook_render_identities(&b)?;
    assert!(same_stored_render_inputs(&left[&1],&right[&1]));
    archive(&b,&changed(&base,"docProps/core.xml"),true)?;
    assert_ne!(left[&1].sheet_render_hash,workbook_render_identities(&b)?[&1].sheet_render_hash);
    assert!(super::super::artifact_cache::assert_shrink_invariants(&a,&b,&dir.0.join("audit")).is_err());
    Ok(())
}
#[test]
fn visual_styles_strings_theme_and_workbook_are_render_inputs() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?; let a = dir.0.join("a.xlsx"); let b = dir.0.join("b.xlsx"); let base = parts();
    archive(&a, &base, false)?; let left = workbook_render_identities(&a)?;
    for name in ["xl/styles.xml", "xl/sharedStrings.xml", "xl/theme/theme1.xml", "xl/workbook.xml"] {
        archive(&b, &changed(&base, name), true)?; let right = workbook_render_identities(&b)?;
        for i in [1,2] { assert_ne!(left[&i].sheet_render_hash, right[&i].sheet_render_hash, "{name}"); }
    }
    Ok(())
}
#[test]
fn visual_nonworksheet_asset_changes_invalidate_the_guarded_global_context() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?; let a = dir.0.join("a.xlsx"); let b = dir.0.join("b.xlsx"); let base = parts();
    archive(&a, &base, false)?; let left = workbook_render_identities(&a)?;
    for name in ["xl/drawings/drawing1.xml", "xl/charts/chart1.xml", "xl/media/image1.png", "xl/drawings/_rels/drawing1.xml.rels"] {
        archive(&b, &changed(&base, name), true)?; let right = workbook_render_identities(&b)?;
        assert_ne!(left[&1].sheet_render_hash, right[&1].sheet_render_hash, "{name}");
        assert_ne!(left[&2].sheet_render_hash, right[&2].sheet_render_hash, "{name}");
    }
    Ok(())
}
#[test]
fn visual_cf_and_linked_charts_need_context_but_plain_cached_formula_does_not() -> anyhow::Result<()> {
    let dir=FixtureDirectory::new()?;let a=dir.0.join("a.xlsx");let b=dir.0.join("b.xlsx");
    let mut plain=plain_parts();
    plain.insert("xl/worksheets/sheet1.xml".into(),b"<worksheet><sheetData><row r='1'><c r='A1'><f>Other!A1</f><v>5</v></c></row></sheetData></worksheet>".to_vec());
    archive(&a,&plain,false)?;archive(&b,&changed(&plain,"xl/worksheets/sheet2.xml"),false)?;
    assert!(same_stored_render_inputs(&workbook_render_identities(&a)?[&1],&workbook_render_identities(&b)?[&1]));
    for (name,xml) in [
        ("xl/worksheets/sheet1.xml","<worksheet><sheetData/><conditionalFormatting/></worksheet>"),
        ("xl/charts/chart1.xml","<chartSpace><f>Other!$A$1:$A$2</f></chartSpace>"),
    ] {
        let mut base=parts();base.insert(name.into(),xml.as_bytes().to_vec());archive(&a,&base,false)?;
        archive(&b,&changed(&base,"xl/worksheets/sheet2.xml"),true)?;
        let left=workbook_render_identities(&a)?;let right=workbook_render_identities(&b)?;
        assert!(!same_stored_render_inputs(&left[&1],&right[&1]));
    }
    Ok(())
}
#[test]
fn visual_runtime_inputs_are_scoped_to_actual_rendering_behavior() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?;
    let path = dir.0.join("a.xlsx");

    let mut base = parts();
    base.insert(
        "xl/worksheets/sheet1.xml".into(),
        b"<worksheet><f>NOW()</f></worksheet>".to_vec(),
    );
    archive(&path, &base, false)?;
    assert!(!workbook_render_identities(&path)?[&1].render_cacheable);

    base = parts();
    base.insert(
        "xl/drawings/_rels/drawing1.xml.rels".into(),
        br#"<Relationships><Relationship Id="p" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.test/a.png" TargetMode="External"/></Relationships>"#.to_vec(),
    );
    archive(&path, &base, false)?;
    assert!(!workbook_render_identities(&path)?[&1].render_cacheable);

    assert!(!visual_external_relationship_requires_runtime(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath"
    ));
    assert!(visual_external_relationship_requires_runtime(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
    ));

    base = plain_parts();
    base.insert(
        "xl/externalLinks/externalLink1.xml".into(),
        b"<externalLink/>".to_vec(),
    );
    base.insert(
        "xl/externalLinks/_rels/externalLink1.xml.rels".into(),
        br#"<Relationships><Relationship Id="x" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath" Target="file:///C:/source.xlsx" TargetMode="External"/></Relationships>"#.to_vec(),
    );
    archive(&path, &base, false)?;
    assert!(
        workbook_render_identities(&path)?
            .values()
            .all(|identity| identity.render_cacheable),
        "disabled external-link updates must not invalidate every stored-state screenshot"
    );

    Ok(())
}
#[test]
fn visual_relationship_parser_accepts_quotes_prefixes_and_entities() -> anyhow::Result<()> {
    let rels = visual_relationships(br#"<?xml version='1.0'?><p:Relationships xmlns:p='urn:fixture'><!-- <Relationship bad/> --><p:Relationship Target='../media/a&amp;b.png' Id='x' Type='image'/></p:Relationships>"#)?;
    assert_eq!(rels.len(),1); assert_eq!(rels[0].target,"../media/a&b.png");
    assert_eq!(visual_resolve_target("xl/drawings/drawing1.xml", "../media/a%20b.png")?, "xl/media/a b.png");
    for input in ["../../escape", "https://example.test/x", "../a%2fb", "bad%", "x#fragment"] {
        assert!(visual_resolve_target("xl/workbook.xml", input).is_err(), "{input}");
    }
    for xml in ["garbage", "<Relationships>", "<Relationships><Unknown/></Relationships>", "<!DOCTYPE x><Relationships/>",
        "<Relationships><Relationship Id='x' Target='a'/></Relationships>",
        "<Relationships><Relationship Id='x' Type='i' Target='a'/><Relationship Id='x' Type='i' Target='b'/></Relationships>"] {
        assert!(visual_relationships(xml.as_bytes()).is_err(), "{xml}");
    }
    Ok(())
}
#[test]
fn visual_cycles_terminate_and_dangling_relationships_disable_reuse() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?; let path = dir.0.join("a.xlsx"); let mut base = parts();
    base.insert("xl/charts/_rels/chart1.xml.rels".into(), br#"<Relationships><Relationship Id="d" Type="drawing" Target="../drawings/drawing1.xml"/></Relationships>"#.to_vec());
    archive(&path,&base,false)?; assert!(workbook_render_identities(&path)?[&1].render_cacheable);
    base.remove("xl/media/image1.png"); archive(&path,&base,false)?;
    assert!(!workbook_render_identities(&path)?[&1].render_cacheable);
    Ok(())
}
#[test]
fn visual_actual_png_reuse_across_different_workbook_hashes() -> anyhow::Result<()> {
    let dir = FixtureDirectory::new()?; let a=dir.0.join("a.xlsx"); let b=dir.0.join("b.xlsx"); let base=plain_parts();
    archive(&a,&base,false)?; archive(&b,&changed(&base,"xl/worksheets/sheet2.xml"),true)?;
    let cache_root=dir.0.join("cache"); let ca=WorkbookScreenshotCache::new(&a,&cache_root,"range")?;
    let cb=WorkbookScreenshotCache::new(&b,&cache_root,"range")?;
    let png=dir.0.join("raw.png"); image::RgbaImage::from_pixel(20,10,image::Rgba([250,60,10,255])).save(&png)?;
    let manifest=dir.0.join("captures.json");
    fs::write(&manifest,serde_json::to_vec(&serde_json::json!([{
        "sheet_index":1,"sheet_name":"Data","original_image":png,"capture_range":"A1:D6","capture_strategy":"test",
        "capture_info":{"strategy":"test","address":"A1:D6","start_row":1,"start_column":1,"rows":6,"columns":4},"tiles":[]
    }]))?)?;
    update_workbook_screenshot_cache(&manifest,&ca,true)?;
    assert_ne!(ca.workbook_hash,cb.workbook_hash);
    let left=ca.valid_current_sheet_captures()?; let right=cb.valid_current_sheet_captures()?;
    assert_eq!(right.len(),1); assert_eq!(left[&1].image,right[&1].image);
    // A global style change must not reuse this image.
    archive(&b,&changed(&base,"xl/styles.xml"),true)?;
    let invalidated=WorkbookScreenshotCache::new(&b,&cache_root,"range")?;
    assert!(invalidated.valid_current_sheet_captures()?.is_empty());
    Ok(())
}
#[test]
fn visual_requested_geometry_is_scoped_to_its_sheet() -> anyhow::Result<()> {
    let dir=FixtureDirectory::new()?; let path=dir.0.join("a.xlsx"); archive(&path,&parts(),false)?;
    let a=WorkbookScreenshotCache::new(&path,&dir.0,"full-a")?.with_range_limits(&[bounds(1,6,4),bounds(2,4,4)])?;
    let b=WorkbookScreenshotCache::new(&path,&dir.0,"full-b")?.with_range_limits(&[bounds(1,6,4),bounds(2,100,10)])?;
    assert_eq!(shared_sheet_screenshot_manifest_path(&a,1,&a.sheet_identities[&1]),shared_sheet_screenshot_manifest_path(&b,1,&b.sheet_identities[&1]));
    assert_ne!(shared_sheet_screenshot_manifest_path(&a,2,&a.sheet_identities[&2]),shared_sheet_screenshot_manifest_path(&b,2,&b.sheet_identities[&2]));
    Ok(())
}

fn plain_parts() -> BTreeMap<String,Vec<u8>> {
    let mut p=parts();
    p.insert("xl/worksheets/sheet1.xml".into(),b"<worksheet><sheetData><row r='1'><c r='A1' t='s'><v>0</v></c></row></sheetData></worksheet>".to_vec());p
}
#[test]
fn visual_theme_extensions_do_not_disable_all_caching() -> anyhow::Result<()> {
    let dir=FixtureDirectory::new()?;let a=dir.0.join("a.xlsx");let mut p=plain_parts();
    p.insert("xl/theme/theme1.xml".into(),b"<theme><extLst><ext uri='fixture'/></extLst></theme>".to_vec());archive(&a,&p,false)?;
    let identities=workbook_render_identities(&a)?;
    assert!(identities.values().all(|i|i.render_cacheable));assert!(identities[&1].render_fallback_reason.is_none());Ok(())
}
#[test]
fn visual_calc_chain_is_not_a_stored_render_input() -> anyhow::Result<()> {
    let dir=FixtureDirectory::new()?;let a=dir.0.join("a.xlsx");let b=dir.0.join("b.xlsx");let mut p=plain_parts();
    p.insert("xl/calcChain.xml".into(),b"<calcChain/>".to_vec());archive(&a,&p,false)?;
    p.insert("xl/calcChain.xml".into(),b"<calcChain><c r='A1' i='1'/></calcChain>".to_vec());archive(&b,&p,false)?;
    assert!(same_stored_render_inputs(&workbook_render_identities(&a)?[&1],&workbook_render_identities(&b)?[&1]));
    assert!(super::super::artifact_cache::assert_shrink_invariants(&a,&b,&dir.0.join("audit")).is_ok());Ok(())
}
