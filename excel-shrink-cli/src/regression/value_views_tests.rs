//! Native Rust tests. These tests do not start Excel or PowerShell.
use super::*;
use super::views::{Cells, SharedStrings, compare_sheet_views};
use anyhow::Result;
use std::{collections::HashSet, fs, io::Write, path::{Path, PathBuf}, time::Instant};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n=NEXT.fetch_add(1, Ordering::Relaxed);
        let time=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("excel-value-v9-{}-{time}-{n}",std::process::id()));
        fs::create_dir_all(&path).unwrap(); Self(path)
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
fn sheet(cells: &str) -> Vec<u8> { format!("<worksheet><sheetData><row r=\"1\">{cells}</row></sheetData></worksheet>").into_bytes() }
fn sst(text: &str) -> SharedStrings { SharedStrings::new(text.as_bytes().to_vec()).unwrap() }
fn compare_xml(left:&[u8], right:&[u8], ls:&SharedStrings, rs:&SharedStrings, policy:MissingCellPolicy, formulas:bool, unordered:bool) -> Result<Vec<CellValueDifference>> {
    let mut differences=Vec::new(); compare_sheet_views("xl/worksheets/sheet1.xml",left,right,ls,rs,policy,formulas,unordered,&mut differences)?; Ok(differences)
}
fn scalar_diff(a:&str,b:&str) -> Result<Vec<CellValueDifference>> {
    let strings=sst(""); compare_xml(&sheet(a),&sheet(b),&strings,&strings,MissingCellPolicy::strict(),true,false)
}
fn workbook(path:&Path, xml:&[u8], strings:Option<&str>) -> Result<()> {
    let mut writer=zip::ZipWriter::new(fs::File::create(path)?);
    let opt=zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name,bytes) in [
        ("xl/workbook.xml", br#"<workbook><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#.as_slice()),
        ("xl/_rels/workbook.xml.rels",br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.as_slice()),
        ("xl/worksheets/sheet1.xml",xml),
    ] { writer.start_file(name,opt)?; writer.write_all(bytes)?; }
    if let Some(strings)=strings { writer.start_file("xl/sharedStrings.xml",opt)?; writer.write_all(strings.as_bytes())?; }
    writer.finish()?; Ok(())
}

#[test]
fn changed_rows_compact_large_recalculation_without_difference_strings() -> Result<()> {
    let fixture = Fixture::new();
    let left = fixture.0.join("stored.xlsx");
    let right = fixture.0.join("recalculated.xlsx");
    for (path, value) in [(&left, "1"), (&right, "2")] {
        let mut xml = String::from("<worksheet><sheetData>");
        for row in 1..=50_000 {
            xml.push_str(&format!("<row r='{row}'><c r='A{row}'><f>RAND()</f><v>{value}</v></c><c r='B{row}'><v>{value}</v></c></row>"));
        }
        xml.push_str("</sheetData></worksheet>");
        workbook(path, xml.as_bytes(), None)?;
    }
    let wanted = HashSet::from(["xl/worksheets/sheet1.xml".to_owned()]);
    let rows = compare_workbook_changed_rows(&left, &right, &wanted)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows["xl/worksheets/sheet1.xml"], vec![[0, 50_000]]);
    assert!(compare_workbook_changed_rows(&left, &left, &wanted)?.is_empty());
    Ok(())
}

#[test]
fn changed_rows_keep_gaps_and_do_not_exempt_hidden_or_whitespace_removals() -> Result<()> {
    let fixture = Fixture::new();
    let left = fixture.0.join("before.xlsx");
    let right = fixture.0.join("after.xlsx");
    workbook(&left, br#"<worksheet><sheetData>
      <row r="1"><c r="A1" t="inlineStr"><is><t> </t></is></c></row>
      <row r="2"><c r="A2"><v>4</v></c></row>
      <row r="3" hidden="1"><c r="A3"><v>9</v></c><c r="B3"><v>8</v></c></row>
    </sheetData></worksheet>"#, None)?;
    workbook(&right, br#"<worksheet><sheetData><row r="2"><c r="A2"><v>4</v></c></row></sheetData></worksheet>"#, None)?;
    let wanted = HashSet::from(["xl/worksheets/sheet1.xml".to_owned()]);
    assert_eq!(compare_workbook_changed_rows(&left, &right, &wanted)?["xl/worksheets/sheet1.xml"], vec![[0, 1], [2, 3]]);
    let missing = HashSet::from(["xl/worksheets/missing.xml".to_owned()]);
    assert!(compare_workbook_changed_rows(&left, &right, &missing).is_err());
    workbook(&right, &sheet("<c r='B1'><v>1</v></c><c r='A1'><v>2</v></c>"), None)?;
    assert!(compare_workbook_changed_rows(&left, &right, &wanted).is_err());
    Ok(())
}

#[test]
fn v9_scalar_bytes_and_sparse_empty_cells() -> Result<()> {
    assert!(scalar_diff(r#"<c r="A1"/><c r="B1"><v>42</v></c><c r="C1" s="99"/>"#,r#"<c r="B1" t="n"><v>42</v></c>"#)?.is_empty());
    assert_eq!(scalar_diff(r#"<c r="A1"><v>1</v></c>"#,r#"<c r="B1"><v>1</v></c>"#)?.len(),2);
    Ok(())
}
#[test]
fn v9_value_type_is_part_of_equality() -> Result<()> {
    for (a,b) in [("n","b"),("e","str"),("n","d")] {
        let left=format!(r#"<c r="A1" t="{a}"><v>1</v></c>"#);
        let right=format!(r#"<c r="A1" t="{b}"><v>1</v></c>"#);
        assert_eq!(scalar_diff(&left,&right)?.len(),1);
    }
    // This is lexical number validation, not rounding/tolerance comparison.
    assert_eq!(scalar_diff(r#"<c r="A1"><v>1</v></c>"#,r#"<c r="A1"><v>1.0</v></c>"#)?.len(),1);
    Ok(())
}
#[test]
fn v9_shared_strings_are_resolved_after_reindexing() -> Result<()> {
    let a=sst("<sst><si><t>alpha</t></si><si><t>beta</t></si></sst>");
    let b=sst("<sst><si><t>beta</t></si><si><t>alpha</t></si></sst>");
    let left=sheet(r#"<c r="A1" t="s"><v>0</v></c>"#);
    let right=sheet(r#"<c r="A1" t="s"><v>01</v></c>"#);
    assert!(compare_xml(&left,&right,&a,&b,MissingCellPolicy::strict(),true,false)?.is_empty());
    Ok(())
}
#[test]
fn v9_shared_inline_and_rich_text_compare_as_text() -> Result<()> {
    let a=sst("<sst><si><r><rPr><b/></rPr><t>A&amp;</t></r><r><t>B&#x1f600;</t></r><rPh sb=\"0\" eb=\"1\"><t>phonetic</t></rPh></si></sst>");
    let b=sst("");
    let left=sheet(r#"<c r="A1" t="s"><v>0</v></c>"#);
    let right=sheet("<c r='A1' t='inlineStr'><is><t>A&#38;B😀</t></is></c>");
    assert!(compare_xml(&left,&right,&a,&b,MissingCellPolicy::strict(),true,false)?.is_empty());
    Ok(())
}
#[test]
fn v9_empty_string_is_not_missing_cell() -> Result<()> {
    assert_eq!(scalar_diff("<c r='A1' t='inlineStr'><is><t/></is></c>","")?.len(),1);
    let a=sst("<sst><si><t/></si></sst>");let b=sst("");
    assert!(compare_xml(&sheet("<c r='A1' t='s'><v>0</v></c>"), &sheet("<c r='A1' t='str'><v></v></c>"), &a,&b,MissingCellPolicy::strict(),true,false)?.is_empty());
    Ok(())
}
#[test]
fn v9_missing_policy_is_asymmetric() -> Result<()> {
    let strings=sst("");
    let left=sheet("<c r='A1' t='inlineStr'><is><t> \t </t></is></c>");
    let missing=sheet("");
    let policy=MissingCellPolicy{allow_whitespace_values:true,allow_hidden_rows_or_columns:false};
    assert!(compare_xml(&left,&missing,&strings,&strings,policy,true,false)?.is_empty());
    assert_eq!(compare_xml(&missing,&left,&strings,&strings,policy,true,false)?.len(),1);
    assert_eq!(compare_xml(&left,&sheet("<c r='A1' t='str'><v>wrong</v></c>"),&strings,&strings,policy,true,false)?.len(),1);
    Ok(())
}
#[test]
fn v9_hidden_row_and_column_removal_policies() -> Result<()> {
    let strings=sst("");let right=sheet("");
    for left in ["<worksheet><sheetData><row r='1' hidden='true'><c r='A1'><v>42</v></c></row></sheetData></worksheet>",
        "<worksheet><cols><col min='1' max='1' hidden='1'/></cols><sheetData><row r='1'><c r='A1'><v>42</v></c></row></sheetData></worksheet>"] {
        assert_eq!(compare_xml(left.as_bytes(),&right,&strings,&strings,MissingCellPolicy::strict(),true,false)?.len(),1);
        assert!(compare_xml(left.as_bytes(),&right,&strings,&strings,MissingCellPolicy{allow_whitespace_values:false,allow_hidden_rows_or_columns:true},true,false)?.is_empty());
    }
    Ok(())
}
#[test]
fn v9_excel_padded_column_widths_allow_complete_value_comparison() -> Result<()> {
    let strings=sst("");
    for width in ["255.5703125", "255.7109375", "256.5", "260"] {
        for definition in [
            format!(r#"<cols><col min="1" max="1" width="{width}"/></cols>"#),
            format!(r#"<sheetFormatPr defaultColWidth="{width}"/>"#),
        ] {
            let left=format!(r#"<worksheet>{definition}<sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData></worksheet>"#);
            assert!(compare_xml(left.as_bytes(),&sheet(r#"<c r="A1"><v>42</v></c>"#),&strings,&strings,MissingCellPolicy::strict(),true,false)?.is_empty());
            assert_eq!(compare_xml(left.as_bytes(),&sheet(r#"<c r="A1"><v>43</v></c>"#),&strings,&strings,MissingCellPolicy::strict(),true,false)?.len(),1);
        }
    }
    Ok(())
}
#[test]
fn v9_invalid_column_widths_still_fail_value_comparison() {
    let strings=sst("");
    for width in ["-0.1", "260.00390625", "65536", "1e308", "NaN", "inf", "-inf"] {
        for definition in [
            format!(r#"<cols><col min="1" max="1" width="{width}"/></cols>"#),
            format!(r#"<sheetFormatPr defaultColWidth="{width}"/>"#),
        ] {
            let invalid=format!("<worksheet>{definition}<sheetData/></worksheet>");
            assert!(compare_xml(invalid.as_bytes(),&sheet(""),&strings,&strings,MissingCellPolicy::strict(),true,false).is_err(),"{invalid}");
            assert!(compare_xml(&sheet(""),invalid.as_bytes(),&strings,&strings,MissingCellPolicy::strict(),true,false).is_err(),"{invalid}");
        }
    }
}
#[test]
fn v9_formula_preservation_is_separate_from_cached_values() -> Result<()> {
    let strings=sst("");
    let a=sheet("<c r='A1'><f>1+1</f><v>2</v></c>");
    let b=sheet("<c r='A1'><f>999</f><v>2</v></c>");
    assert_eq!(compare_xml(&a,&b,&strings,&strings,MissingCellPolicy::strict(),true,false)?.len(),1);
    assert!(compare_xml(&a,&b,&strings,&strings,MissingCellPolicy::strict(),false,false)?.is_empty());
    assert_eq!(scalar_diff("<c r='A1'><f>SUM(B1:B3)</f></c>","")?.len(),1);
    Ok(())
}
#[test]
fn v9_explicit_hidden_policy_can_remove_unreferenced_formula_cell() -> Result<()> {
    let strings=sst("");
    let a=b"<worksheet><sheetData><row r='1' hidden='1'><c r='A1'><f>1</f><v>1</v></c></row></sheetData></worksheet>";
    assert!(compare_xml(a,&sheet(""),&strings,&strings,MissingCellPolicy{allow_whitespace_values:false,allow_hidden_rows_or_columns:true},true,false)?.is_empty());
    Ok(())
}
#[test]
fn v9_prefixes_quotes_comments_and_implicit_addresses() -> Result<()> {
    let strings=sst("");
    let a=b"<x:worksheet xmlns:x='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><x:sheetData><!-- <c r='Z99'/> --><x:row><x:c><x:v>7</x:v></x:c></x:row></x:sheetData></x:worksheet>";
    assert!(compare_xml(a,&sheet("<c r='A1'><v>7</v></c>"),&strings,&strings,MissingCellPolicy::strict(),true,false)?.is_empty());
    Ok(())
}
#[test]
fn v9_malformed_input_does_not_return_a_partial_success() {
    let strings=sst("");
    for xml in ["<worksheet><sheetData><row r='1'><c r='A1'><v>1</c></row></sheetData></worksheet>",
        "<worksheet><sheetData><row r='1'><c r='A1' r='B1'/></row></sheetData></worksheet>",
        "<worksheet><sheetData><row r='1'><c r='A1' t='s'><v>0</v></c></row></sheetData></worksheet>",
        "<worksheet><sheetData><row r='1'><c r='A1'><v><![CDATA[1]]></v></c></row></sheetData></worksheet>",
        "<!DOCTYPE worksheet><worksheet><sheetData/></worksheet>",
        "<worksheet><sheetData><row r='1'><c r='A2'/></row></sheetData></worksheet>"] {
        assert!(compare_xml(xml.as_bytes(),&sheet(""),&strings,&strings,MissingCellPolicy::strict(),true,false).is_err(),"{xml}");
    }
}
#[test]
fn v9_invalid_sst_indexes_fail_and_do_not_become_raw_values() {
    let strings=sst("<sst><si><t>one</t></si></sst>");
    for index in ["1","-1","x","4294967296",""] {
        let xml=sheet(&format!("<c r='A1' t='s'><v>{index}</v></c>"));
        assert!(compare_xml(&xml,&sheet(""),&strings,&strings,MissingCellPolicy::strict(),true,false).is_err());
    }
}
#[test]
fn v9_unordered_fallback_keeps_values_and_rejects_duplicate_addresses() -> Result<()> {
    let strings=sst("");
    let a=sheet("<c r='B1'><v>2</v></c><c r='A1'><v>1</v></c>");
    let b=sheet("<c r='A1'><v>1</v></c><c r='B1'><v>2</v></c>");
    assert!(compare_xml(&a,&b,&strings,&strings,MissingCellPolicy::strict(),true,true)?.is_empty());
    assert!(compare_xml(&a,&b,&strings,&strings,MissingCellPolicy::strict(),true,false).is_err());
    let duplicate=sheet("<c r='B1'/><c r='A1'/><c r='B1'/>");
    assert!(compare_xml(&duplicate,&b,&strings,&strings,MissingCellPolicy::strict(),true,true).is_err());
    Ok(())
}
#[test]
fn v9_values_really_borrow_from_xml_and_sst_buffers() -> Result<()> {
    let xml=sheet("<c r='A1'><v>42</v></c><c r='B1' t='s'><v>0</v></c><c r='C1' t='s'><v>0</v></c>");
    let table=b"<sst><si><t>hello</t></si></sst>".to_vec();let pointer=table.as_ptr() as usize;let end=pointer+table.len();
    let strings=SharedStrings::new(table)?;let mut cells=Cells::new(&xml,&strings)?;
    let number=cells.next()?.unwrap();let raw=number.borrowed_value_bytes();
    assert!((xml.as_ptr() as usize..xml.as_ptr() as usize+xml.len()).contains(&(raw.as_ptr() as usize)));
    let a=cells.next()?.unwrap();let b=cells.next()?.unwrap();
    assert!((pointer..end).contains(&(a.borrowed_value_bytes().as_ptr() as usize)));
    assert_eq!(a.borrowed_value_bytes().as_ptr(),b.borrowed_value_bytes().as_ptr());
    Ok(())
}
#[test]
fn v9_zip_value_cache_invalidates_when_sst_or_sheet_changes() -> Result<()> {
    let f=Fixture::new();let a=f.0.join("a.xlsx");let b=f.0.join("b.xlsx");
    let xml=sheet("<c r='A1' t='s'><v>0</v></c>");
    workbook(&a,&xml,Some("<sst><si><t>first</t></si></sst>"))?;
    workbook(&b,&xml,Some("<sst><si><t>first</t></si></sst>"))?;
    assert_eq!(compare_workbook_values(&a,&b,MissingCellPolicy::strict(),None)?.difference_count,0);
    workbook(&b,&xml,Some("<sst><si><t>second</t></si></sst>"))?;
    assert_eq!(compare_workbook_values(&a,&b,MissingCellPolicy::strict(),None)?.difference_count,1);
    workbook(&b,&sheet("<c r='A1'><v>999</v></c>"),None)?;
    assert_eq!(compare_workbook_values(&a,&b,MissingCellPolicy::strict(),None)?.difference_count,1);
    Ok(())
}
#[test]
fn v9_empty_merged_bounds_and_sheet_filter_are_preserved() -> Result<()> {
    let f=Fixture::new();let a=f.0.join("a.xlsx");let b=f.0.join("b.xlsx");
    workbook(&a,b"<worksheet><sheetData/><mergeCells><mergeCell ref='A1:Z99'/></mergeCells></worksheet>",None)?;
    workbook(&b,b"<worksheet><sheetData/></worksheet>",None)?;
    let report=compare_workbook_values(&a,&b,MissingCellPolicy::strict(),None)?;
    assert_eq!(report.difference_count,0);assert_eq!(report.left_capture_ranges[0].max_cell,"Z99");
    let none=HashSet::new();assert_eq!(compare_workbook_values(&a,&b,MissingCellPolicy::strict(),Some(&none))?.compared_sheets,0);
    Ok(())
}

mod allocations {
    use std::{alloc::{GlobalAlloc,Layout,System},cell::Cell};
    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<u64> = const { Cell::new(0) };
        static BYTES: Cell<u64> = const { Cell::new(0) };
    }
    pub struct Tracking;
    #[global_allocator]
    static ALLOCATOR: Tracking=Tracking;
    fn record(size:usize) { if ACTIVE.try_with(Cell::get).unwrap_or(false) { let _=CALLS.try_with(|v|v.set(v.get()+1));let _=BYTES.try_with(|v|v.set(v.get()+size as u64)); } }
    unsafe impl GlobalAlloc for Tracking {
        unsafe fn alloc(&self,l:Layout)->*mut u8 { record(l.size()); unsafe{System.alloc(l)} }
        unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 { record(l.size()); unsafe{System.alloc_zeroed(l)} }
        unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 {record(n);unsafe{System.realloc(p,l,n)} }
        unsafe fn dealloc(&self,p:*mut u8,l:Layout) { unsafe{System.dealloc(p,l)} }
    }
    pub struct Guard;
    impl Guard {
        pub fn start()->Self { CALLS.with(|v|v.set(0));BYTES.with(|v|v.set(0));ACTIVE.with(|v|v.set(true));Self }
        pub fn counts(&self)->(u64,u64) {(CALLS.with(Cell::get),BYTES.with(Cell::get))}
    }
    impl Drop for Guard { fn drop(&mut self){ACTIVE.with(|v|v.set(false));} }
}
#[test]
fn v9_hot_cell_loop_has_zero_heap_allocations() -> Result<()> {
    let mut a=String::from("<worksheet><sheetData>");let mut b=a.clone();
    for row in 1..=2048 {
        a.push_str(&format!("<row r='{row}'><c r='A{row}'/><c r='B{row}'/><c r='C{row}' t='s'><v>0</v></c><c r='D{row}'><v>42</v></c></row>"));
        b.push_str(&format!("<row r='{row}'><c r='C{row}' t='s'><v>0</v></c><c r='D{row}'><v>42</v></c></row>"));
    }
    a.push_str("</sheetData></worksheet>");b.push_str("</sheetData></worksheet>");
    let strings=sst("<sst><si><t>repeated</t></si></sst>");let mut out=Vec::new();
    compare_sheet_views("sheet",a.as_bytes(),b.as_bytes(),&strings,&strings,MissingCellPolicy::strict(),true,false,&mut out)?;
    let guard=allocations::Guard::start();
    let result=compare_sheet_views("sheet",a.as_bytes(),b.as_bytes(),&strings,&strings,MissingCellPolicy::strict(),true,false,&mut out);
    let (calls,_)=guard.counts();drop(guard);result?;
    assert_eq!(calls,0,"the normal byte-view cell loop must not allocate");assert!(out.is_empty());Ok(())
}
#[test]
#[ignore = "release-mode measurement; optionally set EXCEL_SHRINK_BENCH_LEFT and EXCEL_SHRINK_BENCH_RIGHT"]
fn v9_benchmark_value_comparison() -> Result<()> {
    let fixture=Fixture::new();
    let rows=std::env::var("EXCEL_SHRINK_BENCH_ROWS").ok().and_then(|s|s.parse::<u32>().ok()).unwrap_or(20_000).min(1_048_576);
    let pair=std::env::var_os("EXCEL_SHRINK_BENCH_LEFT").zip(std::env::var_os("EXCEL_SHRINK_BENCH_RIGHT"));
    let (a,b,synthetic)=if let Some((a,b))=pair {(PathBuf::from(a),PathBuf::from(b),false)} else {
        let a=fixture.0.join("bloated.xlsx");let b=fixture.0.join("shrunk.xlsx");
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let mut left=format!("<worksheet><!--{nonce}--><sheetData>");let mut right=left.clone();
        for row in 1..=rows {
            left.push_str(&format!("<row r=\"{row}\">"));right.push_str(&format!("<row r=\"{row}\">"));
            for col in b'A'..=b'P' { left.push_str(&format!("<c r=\"{}{row}\" s=\"1\"/>",col as char)); }
            let data=format!("<c r=\"Q{row}\"><v>{row}</v></c></row>");left.push_str(&data);right.push_str(&data);
        }
        left.push_str("</sheetData></worksheet>");right.push_str("</sheetData></worksheet>");
        workbook(&a,left.as_bytes(),None)?;workbook(&b,right.as_bytes(),None)?;(a,b,true)
    };
    let track=std::env::var("EXCEL_SHRINK_BENCH_ALLOCATIONS").is_ok_and(|v|v == "1");
    let mut samples=Vec::new();
    for mode in ["legacy", "views-cold", "views-warm-1", "views-warm-2", "legacy-repeat"] {
        let guard=track.then(allocations::Guard::start);let started=Instant::now();
        let count=if mode.starts_with("legacy") {super::legacy::compare_workbook_values(&a,&b,super::legacy::MissingCellPolicy::strict(),None)?.difference_count}
            else {compare_workbook_values(&a,&b,MissingCellPolicy::strict(),None)?.difference_count};
        let elapsed=started.elapsed().as_secs_f64()*1000.0;let counts=guard.as_ref().map(allocations::Guard::counts);drop(guard);
        if synthetic {assert_eq!(count,0);}
        samples.push(serde_json::json!({"mode":mode,"elapsed_ms":elapsed,"allocation_calls_current_thread":counts.map(|x|x.0),"allocated_bytes_current_thread":counts.map(|x|x.1),"differences":count}));
    }
    println!("V9_VALUE_BENCHMARK {}",serde_json::json!({"synthetic":synthetic,"rows":rows,"left":a,"right":b,"allocation_instrumentation_enabled":track,"samples":samples}));Ok(())
}
