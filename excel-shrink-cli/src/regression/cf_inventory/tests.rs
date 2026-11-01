use super::*;

fn workbook(path: &Path, xml: &str) -> Result<()> {
    let mut archive = zip::ZipWriter::new(File::create(path)?);
    archive.start_file(
        "xl/worksheets/sheet1.xml",
        zip::write::FileOptions::default(),
    )?;
    archive.write_all(xml.as_bytes())?;
    archive.finish()?;
    Ok(())
}

#[test]
fn raw_evidence_survives_inline_external_and_source_only_storage() -> Result<()> {
    let root = super::super::artifact_cache::unique_path(&std::env::temp_dir(), "cf-evidence")?;
    fs::create_dir_all(&root)?;
    let input = root.join("input.xlsx");
    let nodes: Vec<_> = [17, 20000, 1024*1024+1].into_iter().map(|length| {
        format!("<conditionalFormatting sqref='A1:A9'><!--{}--><cfRule type='cellIs' operator='equal'><formula><![CDATA[0]]></formula></cfRule></conditionalFormatting>", "x".repeat(length))
    }).collect();
    workbook(
        &input,
        &format!("<worksheet><sheetData/>{}</worksheet>", nodes.join("")),
    )?;
    let original = fs::read(&input)?;
    let output = root.join("inventory");
    run(Args {
        input: input.clone(),
        output: output.clone(),
        rows_per_tile: 256,
        columns_per_tile: 32,
        limit: None,
    })?;
    let record: serde_json::Value =
        serde_json::from_str(fs::read_to_string(output.join("sheets.jsonl"))?.trim())?;
    let blocks = record["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["raw_xml"], nodes[0]);
    assert_eq!(
        fs::read_to_string(output.join(blocks[1]["raw_file"].as_str().unwrap()))?,
        nodes[1]
    );
    assert!(blocks[2]["raw_xml"].is_null() && blocks[2]["raw_file"].is_null());
    let source = output.join("sources/00000.xlsx");
    assert_eq!(fs::read(&source)?, original);
    let mut archive = ZipArchive::new(File::open(&source)?)?;
    let mut xml = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")?
        .read_to_string(&mut xml)?;
    for (index, block) in blocks.iter().enumerate() {
        let start = block["start"].as_u64().unwrap() as usize;
        let end = block["end"].as_u64().unwrap() as usize;
        assert_eq!(&xml[start..end], nodes[index]);
    }
    assert_eq!(fs::read(&input)?, original);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_sheet_is_an_error_with_retained_source() -> Result<()> {
    let root = super::super::artifact_cache::unique_path(&std::env::temp_dir(), "cf-malformed")?;
    fs::create_dir_all(&root)?;
    let input = root.join("input.xlsx");
    workbook(&input, "<worksheet><conditionalFormatting sqref='A1'>")?;
    let output = root.join("inventory");
    run(Args {
        input,
        output: output.clone(),
        rows_per_tile: 256,
        columns_per_tile: 32,
        limit: None,
    })?;
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("summary.json"))?)?;
    assert_eq!(summary["status"], "completed_with_errors");
    assert_eq!(summary["errors"], 1);
    assert!(output.join("sources/00000.xlsx").exists());
    fs::remove_dir_all(root)?;
    Ok(())
}
