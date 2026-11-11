//! Correctness checks and an opt-in measurement of the Figure 5 implementation.
use super::*;
use crate::regression::streaming_xml;
use std::{collections::BTreeMap, path::Path, time::Instant};

fn read_cells(input: impl std::io::Read, scratch: &Path) -> Result<Cells> {
    read_cells_with_styles(
        input,
        scratch,
        &super::super::style_identity::StyleIdentity::default(),
    )
}

fn read_cells_with_styles(
    input: impl std::io::Read,
    scratch: &Path,
    styles: &super::super::style_identity::StyleIdentity,
) -> Result<Cells> {
    let mut writer = Writer::new_in(scratch)?;
    streaming_xml::scan(input, |event| writer.event_with_styles(&event, styles))?;
    writer.finish()
}

/// Fragmentation at every input byte must preserve text, CDATA, formula and
/// explicit blank-type evidence. A style-only edit must still be observable.
#[test]
fn fragmented_xml_preserves_cell_evidence_and_style_changes() -> Result<()> {
    let xml = b"<worksheet><c r=\"A1\" t=\"inlineStr\"><is><t><![CDATA[hello & world]]></t></is></c><c r=\"B1\"><f>A2+1</f><v>3</v></c><c r=\"C1\" t=\"s\"/></worksheet>";
    let scratch = std::env::temp_dir();
    let complete = read_cells(xml.as_slice(), &scratch)?;
    let mut scanner = streaming_xml::Scanner::new();
    let mut writer = Writer::new_in(&scratch)?;
    for byte in xml.chunks(1) {
        scanner.feed(byte, |event| writer.event(&event))?;
    }
    scanner.finish()?;
    let fragmented = writer.finish()?;
    assert_eq!(
        complete.0.as_ref().unwrap().digest,
        fragmented.0.as_ref().unwrap().digest
    );
    let mut formulas = Vec::new();
    complete.formula_cells(|cell| {
        formulas.push(cell.address());
        Ok(())
    })?;
    assert_eq!(formulas, ["B1"]);
    assert!(complete.any_input(|cell| cell.address() == "C1" && cell.has_non_numeric_type)?);
    let plain = read_cells(b"<c r=\"A1\"/>".as_slice(), &scratch)?;
    let styled = read_cells(b"<c r=\"A1\" s=\"1\"/>".as_slice(), &scratch)?;
    let mut changes = 0;
    plain.changes(&styled, |_| {
        changes += 1;
        Ok(())
    })?;
    assert_eq!(changes, 1);
    Ok(())
}

#[cfg(windows)]
fn memory() -> Result<serde_json::Value> {
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::GetCurrentProcess,
    };
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&counters) as u32;
    counters.cb = size;
    anyhow::ensure!(
        unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                size,
            )
        } != 0,
        "Cannot read process memory"
    );
    Ok(serde_json::json!({
        "working_set_bytes": counters.WorkingSetSize,
        "peak_working_set_bytes": counters.PeakWorkingSetSize,
        "private_bytes": counters.PrivateUsage,
        "peak_pagefile_bytes": counters.PeakPagefileUsage,
    }))
}

#[cfg(not(windows))]
fn memory() -> Result<serde_json::Value> {
    Ok(serde_json::Value::Null)
}

/// Enumerate absent and two distinct values at four coordinates, including the
/// Z/AA boundary and the last legal cell. The expected merge uses independent
/// coordinate/value maps, without deriving expectations from cell fingerprints.
#[test]
fn all_small_stream_pairs_match_coordinate_value_oracle() -> Result<()> {
    let keys = [
        (9, 26, "Z9"),
        (9, 27, "AA9"),
        (10, 1, "A10"),
        (1_048_576, 16_384, "XFD1048576"),
    ];
    let scratch = std::env::temp_dir();
    let mut variants = Vec::new();
    for variant in 0..81 {
        let mut remaining = variant;
        let mut xml = String::from("<worksheet><sheetData>");
        let mut values = BTreeMap::new();
        for (row, column, address) in keys {
            let value = remaining % 3;
            remaining /= 3;
            if value != 0 {
                xml.push_str(&format!("<c r=\"{address}\"><v>{value}</v></c>"));
                values.insert((row, column), value);
            }
        }
        xml.push_str("</sheetData></worksheet>");
        variants.push((read_cells(xml.as_bytes(), &scratch)?, values));
    }
    for (before, before_values) in &variants {
        for (after, after_values) in &variants {
            let mut actual = Vec::new();
            before.changes(after, |change| {
                actual.push((
                    change.coordinate().key(),
                    change.before.is_some(),
                    change.after.is_some(),
                ));
                Ok(())
            })?;
            let expected: Vec<_> = keys
                .iter()
                .filter_map(|&(row, column, _)| {
                    let key = (row, column);
                    let a = before_values.get(&key);
                    let b = after_values.get(&key);
                    (a != b).then_some((key, a.is_some(), b.is_some()))
                })
                .collect();
            assert_eq!(actual, expected);
        }
    }
    Ok(())
}

#[test]
fn invalid_xml_and_cell_order_fail_without_retaining_scratch() -> Result<()> {
    let scratch =
        std::env::temp_dir().join(format!("cell-stream-invalid-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&scratch)?;
    for xml in [
        "<worksheet><c r=\"A1\"/><c r=\"A1\"/></worksheet>",
        "<worksheet><c r=\"A2\"/><c r=\"A1\"/></worksheet>",
        "<worksheet><c r=\"A1\"><c r=\"B1\"/></c></worksheet>",
        "<worksheet><c r=\"A1\"><v>1</v>",
        "<worksheet><c r=\"A1\"><v>1</c></worksheet>",
        "<worksheet><c/></worksheet>",
        "<worksheet><c r=\"A0\"/></worksheet>",
        "<worksheet><c r=\"XFE1\"/></worksheet>",
        "<worksheet><c r=\"A1048577\"/></worksheet>",
    ] {
        assert!(
            read_cells(xml.as_bytes(), &scratch).is_err(),
            "Accepted {xml}"
        );
        assert_eq!(
            fs::read_dir(&scratch)?.count(),
            0,
            "Leaked scratch for {xml}"
        );
    }
    fs::remove_dir(&scratch)?;
    Ok(())
}

#[test]
fn incomplete_record_and_consumer_failure_are_reported() -> Result<()> {
    let scratch = std::env::temp_dir();
    let before = read_cells(b"<c r=\"A1\"><v>1</v></c>".as_slice(), &scratch)?;
    let after = read_cells(b"<c r=\"A1\"><v>2</v></c>".as_slice(), &scratch)?;
    let error = before
        .changes(&after, |_| anyhow::bail!("consumer failed"))
        .unwrap_err();
    assert!(error.to_string().contains("consumer failed"));
    let path = &after.0.as_ref().unwrap().path;
    OpenOptions::new().write(true).open(path)?.set_len(39)?;
    assert!(before.changes(&after, |_| Ok(())).is_err());
    Ok(())
}

/// Run in a fresh process with CELL_STREAM_BEFORE, CELL_STREAM_AFTER and
/// CELL_STREAM_SCRATCH. The external runner records OS process memory peaks.
/// Setup/input generation stays outside the measured process. No Excel is used.
#[test]
#[ignore = "explicit Figure 5 benchmark; requires input paths"]
fn measure_figure5_stream_comparison() -> Result<()> {
    let before_path = std::env::var("CELL_STREAM_BEFORE")?;
    let after_path = std::env::var("CELL_STREAM_AFTER")?;
    let scratch = PathBuf::from(std::env::var("CELL_STREAM_SCRATCH")?);
    let collect = std::env::var("CELL_STREAM_COLLECT").is_ok_and(|value| value == "1");
    let initial_memory = memory()?;
    let start = Instant::now();
    let styles = match std::env::var("CELL_STREAM_STYLES") {
        Ok(path) => super::super::style_identity::StyleIdentity::read(&fs::read(path)?)?,
        Err(_) => super::super::style_identity::StyleIdentity::default(),
    };
    let styles_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let before = read_cells_with_styles(File::open(&before_path)?, &scratch, &styles)?;
    let before_ms = start.elapsed().as_secs_f64() * 1000.0;
    let before_memory = memory()?;
    let start = Instant::now();
    let after = read_cells_with_styles(File::open(&after_path)?, &scratch, &styles)?;
    let after_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_memory = memory()?;
    let scratch_bytes = fs::metadata(&before.0.as_ref().unwrap().path)?.len()
        + fs::metadata(&after.0.as_ref().unwrap().path)?.len();
    let mut counts = [0usize; 3];
    let mut retained = Vec::new();
    let start = Instant::now();
    before.changes(&after, |change| {
        let index = match (change.before.is_some(), change.after.is_some()) {
            (false, true) => 0,
            (true, false) => 1,
            (true, true) => 2,
            _ => unreachable!("a change has at least one record"),
        };
        counts[index] += 1;
        if collect {
            retained.push(change);
        }
        Ok(())
    })?;
    let merge_ms = start.elapsed().as_secs_f64() * 1000.0;
    let merge_memory = memory()?;
    println!(
        "FIGURE5_RESULT {}",
        serde_json::json!({
            "styles_ms": styles_ms, "before_ms": before_ms, "after_ms": after_ms, "merge_ms": merge_ms,
            "scratch_bytes": scratch_bytes, "added": counts[0], "removed": counts[1],
            "changed": counts[2], "retained_changes": retained.len(),
            "retained_capacity_bytes": retained.capacity() * std::mem::size_of::<CellChange>(),
            "initial_memory": initial_memory, "before_memory": before_memory,
            "after_memory": after_memory, "merge_memory": merge_memory,
        })
    );
    Ok(())
}
