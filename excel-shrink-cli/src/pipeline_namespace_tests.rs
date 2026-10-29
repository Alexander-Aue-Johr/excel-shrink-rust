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

fn preflight<R: Read + Seek>(archive: &mut ZipArchive<R>) -> zip::result::ZipResult<Option<String>> {
    let cli = test_cli(Path::new("probe.xlsx"), Path::new("out"));
    namespace_passthrough_part(archive, &cli)
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
fn root_preflight_ignores_copy_original_custom_xml() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = FileOptions::default();
    writer.start_file("xl/workbook.xml", options).unwrap();
    write!(writer, "<workbook xmlns=\"{MAIN}\"/>").unwrap();
    writer.start_file("customXml/item1.xml", options).unwrap();
    writer
        .write_all(b"\xff\xfe<\0D\0a\0t\0a\0M\0a\0s\0h\0u\0p\0/\0>\0")
        .unwrap();
    let mut archive = ZipArchive::new(writer.finish().unwrap()).unwrap();
    assert_eq!(preflight(&mut archive).unwrap(), None);
    let mut custom_xml = archive.by_name("customXml/item1.xml").unwrap();
    assert!(requires_namespace_root_passthrough(&mut custom_xml).unwrap());
}

#[test]
fn root_preflight_reads_headers_instead_of_stored_sheet_bodies() {
    let mut observations = Vec::new();
    for body_bytes in [4 * 1024 * 1024, 8 * 1024 * 1024] {
        let (mut archive, reads) = counted_archive(archive_with_large_sheet(
            body_bytes,
            CompressionMethod::Stored,
        ));
        assert_eq!(preflight(&mut archive).unwrap(), None);
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
fn late_namespaces_abort_output_without_replacing_prior_results() {
    let cases = [
        (
            "sheet",
            "xl/worksheets/sheet1.xml",
            late_namespace_xml(
                "worksheet",
                &format!("<sheetData><p:row xmlns:p=\"{MAIN}\" r=\"1\"/></sheetData>"),
            ),
        ),
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
            preflight(&mut archive).unwrap(),
            None,
            "{name}: the header probe must not inspect the body"
        );
        drop(archive);
        let output = directory.join(format!("out-{name}"));
        let mut cli = test_cli(&input, &output);
        cli.batch_report = Some(directory.join(format!("report-{name}.json")));
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
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(cli.batch_report.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(report["summary"]["skipped"], 1, "{name}");
        assert_eq!(report["summary"]["failed"], 0, "{name}");
        assert_eq!(report["files"][0]["status"], "skipped", "{name}");
        assert!(report["files"][0]["output_bytes"].is_null(), "{name}");
        assert!(
            report["files"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("unsupported XML namespaces"),
            "{name}"
        );
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
            assert_eq!(preflight(&mut archive).unwrap(), None);
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

fn workbook_members(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut members = std::collections::BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(
            members.insert(entry.name().to_owned(), bytes).is_none(),
            "duplicate ZIP member in {}",
            path.display()
        );
    }
    members
}

fn validation_arguments(input: &Path, output: &Path, flags: &[&str]) -> Vec<OsString> {
    let mut arguments = vec![
        input.as_os_str().to_owned(),
        output.as_os_str().to_owned(),
        OsString::from("--disable-multiprocessing"),
        OsString::from("--force-overwrite"),
        OsString::from("--writer-workers"),
        OsString::from("1"),
    ];
    arguments.extend(flags.iter().map(OsString::from));
    arguments
}

#[test]
#[ignore = "Explicit local workbook validation; requires case JSON and the previous CLI executable"]
fn namespace_real_workbook_validation() {
    #[derive(serde::Deserialize)]
    struct Case {
        path: PathBuf,
        excluded: bool,
    }
    let source_json = PathBuf::from(
        std::env::var_os("EXCEL_SHRINK_NAMESPACE_VALIDATION_JSON")
            .expect("Set EXCEL_SHRINK_NAMESPACE_VALIDATION_JSON to [{path,excluded}, ...]"),
    );
    let previous = PathBuf::from(
        std::env::var_os("EXCEL_SHRINK_PREVIOUS_CLI")
            .expect("Set EXCEL_SHRINK_PREVIOUS_CLI to the previous executable"),
    );
    assert!(
        previous.is_file(),
        "previous CLI is missing: {}",
        previous.display()
    );
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&source_json).unwrap()).unwrap();
    assert!(!cases.is_empty(), "no validation cases were supplied");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output = source_json
        .parent()
        .unwrap()
        .join(format!("namespace-validation-{unique}"));
    std::fs::create_dir_all(&output).unwrap();
    let maximum = [
        "--remove-textless-default-font-representatives",
        "--remove-textless-style-representatives",
        "--remove-textless-column-style-representatives",
        "--remove-unreferenced-whitespace-cells",
    ];
    let mut aggressive = maximum.to_vec();
    aggressive.push("--cell-formula-references-only");
    let profiles: [(&str, &[&str]); 3] = [
        ("default", &[]),
        ("maximum-cleaning", &maximum),
        ("too-aggressive-cleaning", &aggressive),
    ];
    let mut results = Vec::new();
    let mut excluded_profiles = 0;
    let mut supported_profiles = 0;
    let mut supported_cases_with_reduction = 0;
    for (index, case) in cases.iter().enumerate() {
        let original = std::fs::read(&case.path).unwrap();
        let input_members = (!case.excluded).then(|| workbook_members(&case.path));
        let mut any_reduction = false;
        for (profile, flags) in profiles {
            let current_directory = output.join(format!("case-{index}/current/{profile}"));
            let cli = Cli::try_parse_from(
                std::iter::once(OsString::from("excel-shrink")).chain(validation_arguments(
                    &case.path,
                    &current_directory,
                    flags,
                )),
            )
            .unwrap();
            process_files(vec![case.path.clone()], &cli).unwrap();
            let target = current_directory.join(case.path.file_name().unwrap());
            assert!(
                std::fs::read(&case.path).unwrap() == original,
                "source changed: {}",
                case.path.display()
            );
            if case.excluded {
                assert!(
                    !target.exists(),
                    "excluded source produced an output: {} ({profile})",
                    case.path.display()
                );
                assert!(
                    !partial_output_path(&target).exists(),
                    "excluded source left a partial output"
                );
                excluded_profiles += 1;
                results.push(serde_json::json!({"source":case.path,"profile":profile,
                    "excluded":true,"source_unchanged":true,"output_written":false}));
                continue;
            }
            assert!(
                target.is_file(),
                "supported source was excluded: {} ({profile})",
                case.path.display()
            );
            let previous_directory = output.join(format!("case-{index}/previous/{profile}"));
            let prior = std::process::Command::new(&previous)
                .args(validation_arguments(&case.path, &previous_directory, flags))
                .output()
                .unwrap();
            std::fs::create_dir_all(&previous_directory).unwrap();
            std::fs::write(previous_directory.join("stdout.log"), &prior.stdout).unwrap();
            std::fs::write(previous_directory.join("stderr.log"), &prior.stderr).unwrap();
            assert!(
                prior.status.success(),
                "previous CLI failed; see {}",
                previous_directory.display()
            );
            let current_members = workbook_members(&target);
            let prior_target = previous_directory.join(case.path.file_name().unwrap());
            assert!(
                current_members == workbook_members(&prior_target),
                "optimized output member bytes differ: {} ({profile})",
                case.path.display()
            );
            let source_members = input_members.as_ref().unwrap();
            let changed_xml = source_members
                .iter()
                .filter(|(name, bytes)| {
                    name.ends_with(".xml") && current_members.get(*name) != Some(*bytes)
                })
                .count();
            let bytes = std::fs::metadata(&target).unwrap().len();
            any_reduction |= changed_xml > 0 || bytes < original.len() as u64;
            supported_profiles += 1;
            results.push(serde_json::json!({"source":case.path,"profile":profile,
                "excluded":false,"source_unchanged":true,"member_bytes_equal_to_previous":true,
                "original_bytes":original.len(),"output_bytes":bytes,"changed_xml_parts":changed_xml}));
        }
        assert!(
            std::fs::read(&case.path).unwrap() == original,
            "source changed after previous CLI validation"
        );
        if !case.excluded {
            assert!(
                any_reduction,
                "supported fixture did not demonstrate shrinking: {}",
                case.path.display()
            );
            supported_cases_with_reduction += 1;
        }
    }
    let receipt = output.join("receipt.json");
    std::fs::write(
        &receipt,
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"previous_cli":previous,"results":results,
            "comparison":"uncompressed ZIP member names and bytes; entry order ignored"
        }))
        .unwrap(),
    )
    .unwrap();
    println!(
        "{}",
        serde_json::json!({"passed":true,"cases":cases.len(),
        "excluded_profiles":excluded_profiles,"supported_profiles":supported_profiles,
        "supported_cases_with_reduction":supported_cases_with_reduction,"receipt":receipt})
    );
}
