use super::*;
struct Directory(PathBuf);
impl Directory {
    fn new()->Self {
        let p=std::env::temp_dir().join(format!("es-cache-v9-{}-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),NEXT.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir_all(&p).unwrap();Self(p)
    }
}
impl Drop for Directory {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
fn fixture(path:&Path, sheet:&[u8], extra:Option<(&str,&[u8])>, reverse:bool)->Result<()> {
    let mut parts=vec![("xl/workbook.xml",b"<workbook/>".as_slice()),("xl/worksheets/sheet1.xml",sheet)];
    if let Some(p)=extra {parts.push(p);}
    if reverse {parts.reverse();}
    let mut zip=ZipWriter::new(File::create(path)?);
    let option=zip::write::FileOptions::default().compression_method(if reverse{zip::CompressionMethod::Deflated}else{zip::CompressionMethod::Stored});
    for (name,data) in parts {zip.start_file(name,option)?;zip.write_all(data)?;}zip.finish()?;Ok(())
}
#[test]
fn v9_package_key_ignores_zip_container_encoding() ->Result<()> {
    let d=Directory::new();let a=d.0.join("a.xlsx");let b=d.0.join("b.xlsx");
    fixture(&a,b"<worksheet><sheetData/></worksheet>",None,false)?;fixture(&b,b"<worksheet><sheetData/></worksheet>",None,true)?;
    let ma=manifest_in(&a,&d.0)?;let mb=manifest_in(&b,&d.0)?;
    assert_ne!(ma.archive_sha256,mb.archive_sha256);assert_eq!(ma.package_sha256,mb.package_sha256);Ok(())
}
#[test]
fn v9_cache_checksum_and_schema_are_enforced() ->Result<()> {
    let d=Directory::new();let p=d.0.join("record.json");write_record(&p,&vec![1u32,2])?;
    assert_eq!(read_record::<Vec<u32>>(&p),Some(vec![1,2]));
    let mut rec:serde_json::Value=serde_json::from_slice(&fs::read(&p)?)?;rec["data"]=serde_json::json!([9,9]);fs::write(&p,serde_json::to_vec(&rec)?)?;
    assert!(read_record::<Vec<u32>>(&p).is_none());
    write_record(&p,&vec![1u32,2])?;rec=serde_json::from_slice(&fs::read(&p)?)?;rec["schema"]=serde_json::json!(8);fs::write(&p,serde_json::to_vec(&rec)?)?;
    assert!(read_record::<Vec<u32>>(&p).is_none());Ok(())
}
#[test]
fn v9_unexpected_parts_are_preserved_with_an_actual_diff() ->Result<()> {
    let d=Directory::new();let a=d.0.join("a.xlsx");let b=d.0.join("b.xlsx");let sheet=b"<worksheet><sheetData/></worksheet>";
    fixture(&a,sheet,Some(("xl/styles.xml",b"<styleSheet/>")),false)?;
    fixture(&b,sheet,Some(("xl/styles.xml",b"<styleSheet><changed/></styleSheet>")),false)?;
    let report=d.0.join("report");assert!(assert_shrink_invariants(&a,&b,&report).is_err());
    let changes:Vec<PackageChange>=serde_json::from_slice(&fs::read(report.join("package-changes.json"))?)?;
    assert_eq!(changes.len(),1);assert_eq!(changes[0].part,"xl/styles.xml");
    let diff=changes[0].diff_directory.as_ref().unwrap();
    assert_eq!(fs::read(diff.join("before.bin"))?,b"<styleSheet/>");
    assert!(fs::read_to_string(diff.join("diff.txt"))?.contains("changed"));Ok(())
}
#[test]
fn v9_worksheet_addition_is_not_an_allowed_shrink() ->Result<()> {
    let d=Directory::new();let a=d.0.join("a.xlsx");let b=d.0.join("b.xlsx");let sheet=b"<worksheet><sheetData/></worksheet>";
    fixture(&a,sheet,None,false)?;fixture(&b,sheet,Some(("xl/worksheets/sheet2.xml",sheet)),false)?;
    assert!(assert_shrink_invariants(&a,&b,&d.0.join("report")).is_err());Ok(())
}
#[test]
fn v9_snapshots_are_not_writable_hardlinks_to_sources() ->Result<()> {
    let d=Directory::new();let a=d.0.join("a.xlsx");fixture(&a,b"<worksheet><sheetData/></worksheet>",None,false)?;
    let saved=snapshot_in(&a,&d.0.join("cache"))?;let before=fs::read(&saved.path)?;
    fs::write(&a,b"mutated source")?;assert_eq!(fs::read(&saved.path)?,before);Ok(())
}
#[test]
fn v9_container_only_stage_mutations_are_not_silently_ignored() ->Result<()> {
    let d=Directory::new();let a=d.0.join("a.xlsx");let b=d.0.join("b.xlsx");let sheet=b"<worksheet><sheetData/></worksheet>";
    fixture(&a,sheet,None,false)?;fixture(&b,sheet,None,true)?;
    let saved=snapshot_in(&a,&d.0.join("cache"))?;
    assert!(assert_snapshot_unchanged(&saved,&b,&d.0.join("report")).is_err());
    let report:serde_json::Value=serde_json::from_slice(&fs::read(d.0.join("report/mutations.json"))?)?;
    assert_eq!(report["container_only"],true);Ok(())
}
#[test]
fn v9_only_recognized_pure_calculation_inputs_are_reusable() {
    for formula in ["SUM(A1:A9)","IF(A1,ROUND(B2,2),0)","Other!A1+3"] {
        let x=format!("<worksheet><f>{formula}</f></worksheet>");assert_eq!(calculation_certificate("xl/worksheets/sheet1.xml",x.as_bytes()),(true,true));
    }
    for formula in ["NOW()","TODAY()","RAND()","INDIRECT(A1)","CustomUdf(A1)","[book.xlsx]Sheet!A1","_xlfn.RANDARRAY(3,3)","RAND&#40;)","ÜSUM(A1)"] {
        let x=format!("<worksheet><f>{formula}</f></worksheet>");assert!(!calculation_certificate("xl/worksheets/sheet1.xml",x.as_bytes()).0,"{formula}");
    }
    assert!(!calculation_certificate("xl/workbook.xml",b"<workbook><calcPr iterate='1'/></workbook>").0);
    assert!(!calculation_certificate("xl/connections.xml",b"<connections/>").0);
    assert!(!calculation_certificate("xl/worksheets/sheet1.xml",b"<worksheet><f t='dataTable'/></worksheet>").0);
}
#[test]
fn v9_repair_is_idempotent_and_changes_workbook_xml_only() ->Result<()> {
    let d=Directory::new();let input=d.0.join("original.xlsx");
    let workbook=br#"<workbook><definedNames><definedName name="Print_Area" localSheetId="0">Data!$A$1</definedName><definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1</definedName></definedNames></workbook>"#;
    let mut writer=ZipWriter::new(File::create(&input)?);let opt=zip::write::FileOptions::default();
    writer.start_file("xl/workbook.xml",opt)?;writer.write_all(workbook)?;
    writer.start_file("xl/worksheets/sheet1.xml",opt)?;writer.write_all(b"<worksheet><sheetData/></worksheet>")?;
    writer.finish()?;
    let original=fs::read(&input)?;let baseline=prepare_baseline(&input)?;let again=prepare_baseline(&input)?;
    assert!(baseline.workbook_xml_changed);assert_eq!(baseline.snapshot.path,again.snapshot.path);
    assert_eq!(original,fs::read(&input)?);
    assert_eq!(member(&input,"xl/worksheets/sheet1.xml")?,member(&baseline.snapshot.path,"xl/worksheets/sheet1.xml")?);
    let cleaned=member(&baseline.snapshot.path,"xl/workbook.xml")?.unwrap();
    assert_eq!(excel_shrink_core::workbook::clean_workbook(&cleaned)?,cleaned);Ok(())
}
