use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::{write::FileOptions, ZipArchive, ZipWriter};

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(path: &Path) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    let rows: String = (1..100)
        .map(|n| format!("<row r=\"{n}\"><c r=\"A{n}\"><v>{n}</v></c><c r=\"B{n}\"/></row>"))
        .collect();
    for (name, data) in [
        ("xl/workbook.xml", "<workbook><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Lookup\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>".to_owned()),
        ("xl/_rels/workbook.xml.rels", "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Target=\"worksheets/sheet2.xml\"/></Relationships>".to_owned()),
        ("xl/worksheets/sheet1.xml", format!("<worksheet><sheetData>{rows}<row r=\"100\" hidden=\"1\"><c r=\"A100\"><v>42</v></c></row></sheetData></worksheet>")),
        ("xl/worksheets/sheet2.xml", "<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><f>Data!A100</f><v>42</v></c></row></sheetData></worksheet>".to_owned()),
    ] {
        zip.start_file(name, FileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn entries(path: &Path) -> HashMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|index| {
            let mut file = zip.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}

#[test]
fn parallel_cleanup_trace_preserves_output_and_records_balanced_resources_and_handoffs() {
    let root = TempDir(std::env::temp_dir().join(format!(
            "excel-shrink-profile-integration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    std::fs::create_dir_all(root.0.join("input")).unwrap();
    fixture(&root.0.join("input/first.xlsx"));
    fixture(&root.0.join("input/second.xlsx"));
    for (output, trace, chunk_size) in [
        ("plain", false, "512"),
        ("traced", true, "512"),
        ("unchunked", false, "0"),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_excel-shrink-cli"));
        command
            .current_dir(&root.0)
            .env("RAYON_NUM_THREADS", "4")
            .args([
                "input",
                output,
                "--parallel-range-chunk-size",
                chunk_size,
                "--max-active-workbooks",
                "1",
                "--writer-workers",
                "1",
                "--console-log-level",
                "error",
            ]);
        if trace {
            command.args(["--trace-file", "trace.json"]);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for name in ["first.xlsx", "second.xlsx"] {
        assert_eq!(
            entries(&root.0.join("plain").join(name)),
            entries(&root.0.join("traced").join(name))
        );
        assert_eq!(
            entries(&root.0.join("plain").join(name)),
            entries(&root.0.join("unchunked").join(name))
        );
    }
    let trace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.0.join("trace.json")).unwrap()).unwrap();
    assert_eq!(trace["otherData"]["dropped_events"], 0);
    let mut normalized = trace["traceEvents"].as_array().unwrap().clone();
    let chunks: Vec<_> = normalized
        .iter()
        .filter(|e| e["args"]["phase_name"] == "clean_range_chunk")
        .collect();
    assert!(!chunks.is_empty());
    for chunk in chunks {
        let sheet = chunk["args"]["sheet"].as_str().unwrap();
        let label = sheet.rsplit('/').next().unwrap().trim_end_matches(".xml");
        assert_eq!(chunk["name"], format!("[{label}] clean_range_chunk"));
        assert!(chunk["args"]["chunk"].is_u64());
    }
    for event in &mut normalized {
        if let Some(name) = event["args"]["phase_name"].as_str().map(str::to_owned) {
            event["name"] = name.into();
        }
    }
    let events = &normalized;
    // These workbooks have no calcChain: no address scan or publication is needed.
    for name in [
        "collect_cell_references",
        "publish_cell_references",
        "clean_calc_chain",
    ] {
        assert!(
            !events.iter().any(|e| e["name"] == name),
            "unexpected {name}"
        );
    }
    let starts: HashMap<_, _> = events
        .iter()
        .filter(|e| e["ph"] == "s")
        .map(|e| (e["id"].as_u64().unwrap(), e))
        .collect();
    let ends: HashMap<_, _> = events
        .iter()
        .filter(|e| e["ph"] == "f")
        .map(|e| (e["id"].as_u64().unwrap(), e))
        .collect();
    assert!(!starts.is_empty());
    assert_eq!(starts.len(), ends.len());
    assert!(starts.iter().any(|(id, e)| e["tid"] != ends[id]["tid"]));
    for (id, start) in &starts {
        assert!(start["ts"].as_f64().unwrap() <= ends[id]["ts"].as_f64().unwrap());
    }
    for name in [
        "active_workbooks",
        "writer_pending_jobs",
        "writer_pending_compressed_bytes",
        "initial_results_buffered_bytes",
    ] {
        let values: Vec<_> = events
            .iter()
            .filter(|e| e["ph"] == "C" && e["name"] == name)
            .map(|e| e["args"]["value"].as_u64().unwrap())
            .collect();
        assert!(!values.is_empty(), "{name}");
        assert_eq!(values.last(), Some(&0), "{name}");
    }
    for name in [
        "clean_range_chunk",
        "writer_wait_for_job",
        "scan_sheet_references",
        "sheet_timing",
        "row_inspect_header",
        "row_scan_and_clean_cells",
        "row_render_xml",
        "row_append_output",
        "sampled_cell_work",
        "range_row_samples",
        "preprocess_read_decompress",
        "preprocess_append_input",
        "preprocess_find_sheet_data_end",
        "preprocess_sheet_data_prefix",
        "preprocess_check_cdata",
        "preprocess_append_result",
        "preprocess_retain_buffer_suffix",
    ] {
        assert!(events.iter().any(|e| e["name"] == name), "missing {name}");
    }
    let reads: Vec<_> = events
        .iter()
        .filter(|event| event["name"] == "preprocess_read_decompress")
        .collect();
    assert!(!reads.is_empty());
    assert!(reads
        .iter()
        .any(|event| event["args"]["bytes_read"].as_u64().unwrap() > 0));
    for event in reads {
        let requested = event["args"]["requested_bytes"].as_u64().unwrap();
        let actual = event["args"]["bytes_read"].as_u64().unwrap();
        assert!(actual <= requested);
    }
    let sampled_cells: u64 = events
        .iter()
        .filter(|e| e["name"] == "sampled_cell_work")
        .map(|e| e["args"]["cells"].as_u64().unwrap())
        .sum();
    assert!(sampled_cells > 0);
    let samples: Vec<_> = events
        .iter()
        .filter(|e| e["name"] == "range_row_samples")
        .collect();
    let total_rows: u64 = samples
        .iter()
        .map(|e| e["args"]["total_rows"].as_u64().unwrap())
        .sum();
    let sampled_rows: u64 = samples
        .iter()
        .map(|e| e["args"]["sampled_rows"].as_u64().unwrap())
        .sum();
    assert!(sampled_rows > 0 && sampled_rows < total_rows);
    let drops_before: HashMap<_, _> = events
        .iter()
        .filter(|e| e["name"] == "before_drop")
        .map(|e| (e["args"]["drop_id"].as_u64().unwrap(), e))
        .collect();
    let drops_after: HashMap<_, _> = events
        .iter()
        .filter(|e| e["name"] == "after_drop")
        .map(|e| (e["args"]["drop_id"].as_u64().unwrap(), e))
        .collect();
    assert!(!drops_before.is_empty());
    assert_eq!(drops_before.len(), drops_after.len());
    for (id, before) in &drops_before {
        let after = drops_after[id];
        assert_eq!(before["args"]["resource"], after["args"]["resource"]);
        assert_eq!(before["tid"], after["tid"]);
        assert!(before["ts"].as_f64().unwrap() <= after["ts"].as_f64().unwrap());
        assert!(after["args"]["elapsed_ns"].as_u64().is_some());
    }
    for resource in [
        "workbook.archive",
        "writer.buffered_output",
        "sheet.metrics",
        "sheet.metrics.kept_cell_states",
        "sheet.metrics.deleted_cell_count",
        "batch.analysis_sender",
    ] {
        assert!(
            drops_after
                .values()
                .any(|e| e["args"]["resource"] == resource),
            "missing {resource}"
        );
    }
    for parent in drops_before
        .values()
        .filter(|e| e["args"]["resource"] == "sheet.metrics")
    {
        let parent_end = drops_after[&parent["args"]["drop_id"].as_u64().unwrap()];
        let fields: Vec<_> = drops_before
            .values()
            .filter(|e| {
                e["tid"] == parent["tid"]
                    && e["ts"].as_f64().unwrap() > parent["ts"].as_f64().unwrap()
                    && e["ts"].as_f64().unwrap() < parent_end["ts"].as_f64().unwrap()
                    && e["args"]["resource"]
                        .as_str()
                        .unwrap()
                        .starts_with("sheet.metrics.")
            })
            .collect();
        assert_eq!(fields.len(), 15);
    }
    // A CLI error after the recorder starts must still produce a complete trace.
    let result = Command::new(env!("CARGO_BIN_EXE_excel-shrink-cli"))
        .current_dir(&root.0)
        .args([
            "regression",
            "smoke",
            "--case-id",
            "does-not-exist",
            "--trace-file",
            "failed.json",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let _: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.0.join("failed.json")).unwrap()).unwrap();
}
