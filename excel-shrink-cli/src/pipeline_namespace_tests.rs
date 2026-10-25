use super::*;
use clap::Parser;
use std::cell::Cell;
use std::ffi::OsString;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use zip::write::FileOptions;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

struct CountedArchive {
    source: Cursor<Vec<u8>>,
    bytes_read: Rc<Cell<u64>>,
}

impl Read for CountedArchive {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let count = self.source.read(output)?;
        self.bytes_read.set(self.bytes_read.get() + count as u64);
        Ok(count)
    }
}

impl Seek for CountedArchive {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.source.seek(position)
    }
}

fn counted_archive(bytes: Vec<u8>) -> (ZipArchive<CountedArchive>, Rc<Cell<u64>>) {
    let bytes_read = Rc::new(Cell::new(0));
    let archive = ZipArchive::new(CountedArchive {
        source: Cursor::new(bytes),
        bytes_read: bytes_read.clone(),
    })
    .unwrap();
    // ZIP directory discovery is common to both algorithms. Count only reads
    // performed after that metadata has been loaded.
    bytes_read.set(0);
    (archive, bytes_read)
}

fn archive_with_large_sheet(body_bytes: usize, compression: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = FileOptions::default()
        .compression_method(compression)
        .compression_level((compression == CompressionMethod::Deflated).then_some(1));
    writer.start_file("xl/workbook.xml", options).unwrap();
    write!(writer, "<workbook xmlns=\"{MAIN}\"><sheets/></workbook>").unwrap();
    writer
        .start_file("xl/worksheets/sheet1.xml", options)
        .unwrap();
    write!(writer, "<worksheet xmlns=\"{MAIN}\"><sheetData>").unwrap();
    let block = [b' '; 64 * 1024];
    let mut remaining = body_bytes;
    while remaining > 0 {
        let count = remaining.min(block.len());
        writer.write_all(&block[..count]).unwrap();
        remaining -= count;
    }
    writer.write_all(b"</sheetData></worksheet>").unwrap();
    writer
        .start_file("xl/_rels/workbook.xml.rels", options)
        .unwrap();
    writer
        .write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#)
        .unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn root_preflight_reads_headers_instead_of_stored_sheet_bodies() {
    let mut observations = Vec::new();
    for body_bytes in [4 * 1024 * 1024, 8 * 1024 * 1024] {
        let (mut archive, reads) = counted_archive(archive_with_large_sheet(
            body_bytes,
            CompressionMethod::Stored,
        ));
        assert_eq!(namespace_passthrough_part(&mut archive).unwrap(), None);
        let inspected = reads.get();
        assert!(
            inspected > 512,
            "the counter did not observe the root reads"
        );
        assert!(
            inspected < 64 * 1024,
            "read {inspected} bytes for three small XML roots"
        );
        observations.push(inspected);
    }
    assert_eq!(
        observations[0], observations[1],
        "root checks must not scale with sheet bodies"
    );
}

fn workbook_xml() -> Vec<u8> {
    format!(
        "<workbook xmlns=\"{MAIN}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Probe\" sheetId=\"1\" r:id=\"rId1\"/></sheets><definedNames><definedName name=\"Print_Area\" localSheetId=\"0\">Probe!$A$1</definedName><definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">Probe!$A$1</definedName></definedNames></workbook>"
    ).into_bytes()
}

fn late_namespace_xml(root: &str, nested: &str) -> Vec<u8> {
    // The root-only classifier must finish before this declaration. Only normal
    // processing of the relevant part should discover it and cancel publication.
    let mut xml = format!("<{root} xmlns=\"{MAIN}\"><!--").into_bytes();
    xml.extend(std::iter::repeat_n(b'x', 8192));
    xml.extend_from_slice(b"-->");
    xml.extend_from_slice(nested.as_bytes());
    xml.extend_from_slice(format!("</{root}>").as_bytes());
    xml
}

fn write_late_namespace_case(path: &Path, changed_part: &str, changed_xml: &[u8]) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let normal_workbook = workbook_xml();
    let entries: &[(&str, &[u8])] = &[
        ("xl/worksheets/sheet1.xml", br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"/><c r="C1"/></row></sheetData></worksheet>"#),
        ("xl/workbook.xml", &normal_workbook),
        ("xl/_rels/workbook.xml.rels", br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#),
        ("xl/styles.xml", br#"<styleSheet><fonts count="1"><font/></fonts><cellXfs count="1"><xf fontId="0" fillId="0" borderId="0"/></cellXfs></styleSheet>"#),
        ("xl/sharedStrings.xml", br#"<sst><si><t>Important text</t></si></sst>"#),
        ("xl/pivotTables/pivotTable1.xml", br#"<pivotTableDefinition><location ref="A1:B2"/></pivotTableDefinition>"#),
    ];
    for (name, ordinary) in entries {
        writer.start_file(*name, FileOptions::default()).unwrap();
        writer
            .write_all(if *name == changed_part {
                changed_xml
            } else {
                ordinary
            })
            .unwrap();
    }
    writer.finish().unwrap();
}

fn test_cli(input: &Path, output: &Path) -> Cli {
    Cli::try_parse_from([
        OsString::from("excel-shrink"),
        input.as_os_str().to_owned(),
        output.as_os_str().to_owned(),
        OsString::from("--force-overwrite"),
        OsString::from("--disable-multiprocessing"),
        OsString::from("--writer-workers"),
        OsString::from("1"),
        OsString::from("--remove-unreferenced-whitespace-cells"),
    ])
    .unwrap()
}

#[test]
fn late_global_namespaces_abort_output_without_replacing_prior_results() {
    let cases = [
        (
            "styles",
            "xl/styles.xml",
            late_namespace_xml(
                "styleSheet",
                &format!("<cellXfs count=\"1\"><p:xf xmlns:p=\"{MAIN}\" fontId=\"0\"/></cellXfs>"),
            ),
        ),
        (
            "workbook",
            "xl/workbook.xml",
            late_namespace_xml(
                "workbook",
                &format!(
                    "<sheets><p:sheet xmlns:p=\"{MAIN}\" name=\"Probe\" sheetId=\"1\"/></sheets>"
                ),
            ),
        ),
        (
            "strings",
            "xl/sharedStrings.xml",
            late_namespace_xml(
                "sst",
                &format!("<si><p:t xmlns:p=\"{MAIN}\">Important text</p:t></si>"),
            ),
        ),
        (
            "pivot",
            "xl/pivotTables/pivotTable1.xml",
            late_namespace_xml(
                "pivotTableDefinition",
                &format!("<p:location xmlns:p=\"{MAIN}\" ref=\"A1:B2\"/>"),
            ),
        ),
    ];
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "excel_shrink_late_namespace_{}_{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    for (name, part, xml) in cases {
        let input = directory.join(format!("{name}.xlsx"));
        write_late_namespace_case(&input, part, &xml);
        let mut archive = ZipArchive::new(File::open(&input).unwrap()).unwrap();
        assert_eq!(
            namespace_passthrough_part(&mut archive).unwrap(),
            None,
            "{name}: the header probe must not inspect the body"
        );
        drop(archive);
        let output = directory.join(format!("out-{name}"));
        let cli = test_cli(&input, &output);
        let target = output.join(input.file_name().unwrap());
        process_files(vec![input.clone()], &cli).unwrap();
        assert!(
            !target.exists(),
            "{name}: late unsupported XML published an output"
        );
        assert!(
            !partial_output_path(&target).exists(),
            "{name}: partial output survived cancellation"
        );
        std::fs::write(&target, b"prior result must survive").unwrap();
        process_files(vec![input], &cli).unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"prior result must survive",
            "{name}"
        );
        assert!(!partial_output_path(&target).exists(), "{name}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

fn old_full_preflight<R: Read + Seek>(archive: &mut ZipArchive<R>) {
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if entry.name().ends_with(".xml") || entry.name().ends_with(".rels") {
            assert!(!requires_namespace_passthrough(&mut entry).unwrap());
        }
    }
}

fn median_probe(bytes: &[u8], full: bool) -> (f64, u64) {
    let mut samples = Vec::new();
    for _ in 0..3 {
        let (mut archive, reads) = counted_archive(bytes.to_vec());
        let start = Instant::now();
        if full {
            old_full_preflight(&mut archive);
        } else {
            assert_eq!(namespace_passthrough_part(&mut archive).unwrap(), None);
        }
        samples.push((start.elapsed().as_secs_f64() * 1000.0, reads.get()));
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    samples[1]
}

#[test]
#[ignore = "Explicit namespace preflight microbenchmark; does not run the corpus or measure total shrink time"]
fn namespace_header_benchmark() {
    for compression in [CompressionMethod::Stored, CompressionMethod::Deflated] {
        let body_bytes = 32 * 1024 * 1024;
        let bytes = archive_with_large_sheet(body_bytes, compression);
        let old = median_probe(&bytes, true);
        let new = median_probe(&bytes, false);
        println!(
            "{}",
            serde_json::json!({
                "benchmark":"namespace_preflight_only", "compression":format!("{compression:?}"),
                "sheet_body_bytes":body_bytes,"zip_bytes":bytes.len(),"median_repeats":3,
                "full_pass":{"milliseconds":old.0,"archive_bytes_read":old.1},
                "root_headers":{"milliseconds":new.0,"archive_bytes_read":new.1}
            })
        );
    }
}
