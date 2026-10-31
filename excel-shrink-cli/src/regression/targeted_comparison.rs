//! Standalone, targeted comparison of one original workbook and one shrink result.
//! The JSON report keeps skipped checks explicit; it is not a full visual proof.

use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use image::GenericImageView;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use zip::ZipArchive;

use super::office_plan::build_workbook_visual_plan;
use super::worksheet_stream::WorksheetStream;
use super::xml_views::{Kind, Reader};

const MAX_CANDIDATES: usize = 64;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Untouched source XLSX; the command works on a copy.
    #[arg(long)]
    original: PathBuf,
    /// XLSX produced by the default shrink profile; the command works on a copy.
    #[arg(long)]
    default: PathBuf,
    /// New directory for the report, screenshots and working copies.
    #[arg(long)]
    output: PathBuf,
    /// PowerShell Excel host script (defaults to the repository copy).
    #[arg(long)]
    host_script: Option<PathBuf>,
    /// Existing Observatory group directory to receive live telemetry.
    #[arg(long)]
    observatory_group_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize)]
struct Candidate {
    sheet: String,
    cell: String,
}

#[derive(Serialize)]
struct Finding {
    sheet: String,
    cell: String,
    range: String,
    before_recalc_differs: bool,
    after_shrink_recalc_differs: bool,
    also_changed_by_original_recalc: bool,
    before_recalc_geometry_differs: bool,
    after_shrink_recalc_geometry_differs: bool,
    original_save_xml_changed: bool,
    original_before_png: PathBuf,
    shrink_before_png: PathBuf,
    shrink_recalc_png: PathBuf,
    original_recalc_png: PathBuf,
    original_open_geometry: Value,
    shrink_open_geometry: Value,
    shrink_recalc_geometry: Value,
    original_recalc_geometry: Value,
}

#[derive(Serialize)]
struct Report {
    status: String,
    check_performance: &'static str,
    candidate_count: usize,
    findings: Vec<Finding>,
    limitations: Vec<String>,
    phases: Vec<String>,
    excel_pids: BTreeMap<String, u32>,
    capture_counts: BTreeMap<String, usize>,
}

/// Starts the standalone targeted comparison and records failures as JSON.
///
/// The command needs no Observatory process. When given an existing group
/// directory it also publishes live telemetry there.
///
/// # Errors
///
/// Returns an error when inputs are invalid, the candidate bound is exceeded,
/// Excel automation fails, or evidence cannot be written. Existing inputs are
/// never modified.
pub(crate) fn run(args: Args) -> Result<()> {
    let outcome = compare_workbooks(&args);
    if let Err(error) = &outcome {
        if args.output.is_dir() {
            let _ = record_failure(&args, error);
        }
    }
    outcome
}

fn record_failure(args: &Args, error: &anyhow::Error) -> Result<()> {
    let message = format!("{error:#}");
    fs::write(
        args.output.join("failure.json"),
        serde_json::to_vec_pretty(&json!({"status":"failed", "error":message}))?,
    )?;
    let telemetry_path = args.output.join("telemetry.json");
    if telemetry_path.exists() {
        let mut telemetry: Value = serde_json::from_slice(&fs::read(&telemetry_path)?)?;
        telemetry["activity"] = json!("failed");
        telemetry["error"] = json!(message);
        telemetry["report"]["status"] = json!("failed");
        let bytes = serde_json::to_vec_pretty(&telemetry)?;
        fs::write(&telemetry_path, &bytes)?;
        if let Some(group_dir) = &args.observatory_group_dir {
            fs::write(group_dir.join("targeted-comparison.json"), bytes)?;
        }
    }
    Ok(())
}

/// Compares bounded changed-cell regions while both Excel workbooks stay open.
///
/// Copies both inputs, discovers XML candidates, opens each copy once, saves
/// them, and adds save-only XML changes. It captures those regions in the open
/// original and shrink workbooks, rebuilds shrink, then rebuilds the original.
/// A change also seen after original recalculation gets a separate result flag.
/// Screenshots and rendered cell positions are retained as evidence. Other XML
/// changes and candidates introduced only by rebuilding are not yet covered;
/// the report names those limits. Check Performance is optional and currently
/// reported as unavailable, never inferred from the other two workbooks.
///
/// Each call below represents one phase; the helpers own the XML, COM and
/// image details. See [`run`] for failure reporting at the CLI boundary.
fn compare_workbooks(args: &Args) -> Result<()> {
    let paths = prepare_working_copies(args)?;
    let mut report = new_report();
    let mut candidates = discover_candidates(&paths.original, &paths.default)?;
    report.phases.push("initial XML compared".into());
    if candidates.is_empty() {
        report.status = "no_cell_candidates_with_limitations".into();
        publish(args, &report, "complete", &candidates)?;
        return Ok(());
    }
    publish(args, &report, "opening", &candidates)?;

    let mut excel = ExcelHost::start(args.host_script.as_deref())?;
    report.excel_pids = open_workbooks(&mut excel, &paths)?;
    report.phases.push("original and shrink opened".into());
    publish(args, &report, "xml", &candidates)?;

    let original_save_changes = save_and_extend_candidates(&mut excel, &paths, &mut candidates)?;
    report.phases.push("Excel save effects inspected".into());
    publish(args, &report, "capturing", &candidates)?;

    let original_before = capture_candidates(
        &mut excel,
        "O",
        &candidates,
        args,
        &mut report,
        "original-before",
    )?;
    let shrink_before = capture_candidates(
        &mut excel,
        "SD",
        &candidates,
        args,
        &mut report,
        "shrink-before",
    )?;
    report.phases.push("initial regions captured".into());
    publish(args, &report, "recalc:SD", &candidates)?;

    excel.call(json!({"kind":"recalculate", "lane":"SD"}))?;
    let shrink_after = capture_candidates(
        &mut excel,
        "SD",
        &candidates,
        args,
        &mut report,
        "shrink-recalc",
    )?;
    report.phases.push("shrink rebuilt and captured".into());
    publish(args, &report, "recalc:O", &candidates)?;

    excel.call(json!({"kind":"recalculate", "lane":"O"}))?;
    let original_after = capture_candidates(
        &mut excel,
        "O",
        &candidates,
        args,
        &mut report,
        "original-recalc",
    )?;
    report.phases.push("original rebuilt and captured".into());
    publish(args, &report, "comparing", &candidates)?;

    record_check_performance_unavailable(&mut report);
    report.findings = compare_evidence(
        &candidates,
        &original_before,
        &shrink_before,
        &shrink_after,
        &original_after,
        &original_save_changes,
    )?;
    report.candidate_count = candidates.len();
    report.status = "completed_with_limitations".into();
    report
        .phases
        .push("images and rendered geometry compared".into());
    publish(args, &report, "complete", &candidates)?;
    Ok(())
}

/// Explicit extension point for the machine with Check Performance installed.
/// No Windows automation is attempted here, and no CP result is fabricated.
fn record_check_performance_unavailable(report: &mut Report) {
    report.check_performance = "unavailable_not_tested";
}

struct WorkingPaths {
    original: PathBuf,
    default: PathBuf,
}

fn prepare_working_copies(args: &Args) -> Result<WorkingPaths> {
    ensure!(
        !args.output.exists(),
        "output directory already exists: {}",
        args.output.display()
    );
    fs::create_dir_all(&args.output)?;
    let original = args.output.join("original.xlsx");
    let default = args.output.join("default.xlsx");
    fs::copy(&args.original, &original).context("copy original")?;
    fs::copy(&args.default, &default).context("copy shrink result")?;
    Ok(WorkingPaths { original, default })
}

fn new_report() -> Report {
    Report {
        status: "running".into(),
        check_performance: "unavailable_not_tested",
        candidate_count: 0,
        findings: Vec::new(),
        limitations: vec![
            "Only changed cell XML is selected; drawing, chart, style and worksheet geometry changes are not yet exhaustively discovered.".into(),
            "Only the first 64 distinct changed cells can be captured; larger cases fail explicitly.".into(),
            "Excel may recalculate during open, even when manual calculation is requested.".into(),
            "Excel save changes found after initial candidate discovery are included, but saved-versus-reopened rendering is not yet checked.".into(),
        ],
        phases: Vec::new(),
        excel_pids: BTreeMap::new(),
        capture_counts: BTreeMap::new(),
    }
}

/// Atomically exposes phase and per-process progress to the optional Observatory group.
fn publish(
    args: &Args,
    report: &Report,
    activity: &str,
    candidates: &BTreeSet<Candidate>,
) -> Result<()> {
    let active_lane = activity.strip_prefix("capture:");
    let lane_status = |lane: &str| -> &str {
        if activity == "complete" {
            "complete"
        } else if activity == "opening" {
            "opening"
        } else if activity == "xml" {
            "xml"
        } else if activity == format!("recalc:{lane}") {
            "recalculating"
        } else if active_lane == Some(lane) {
            "capturing"
        } else {
            "waiting"
        }
    };
    let document = json!({
        "activity": activity,
        "lanes": [
            {"id":"O", "label":"Original", "status": lane_status("O"), "pid":report.excel_pids.get("O"), "captures_done":report.capture_counts.get("O").copied().unwrap_or(0), "captures_total":candidates.len()*2},
            {"id":"SD", "label":"Shrink Default", "status": lane_status("SD"), "pid":report.excel_pids.get("SD"), "captures_done":report.capture_counts.get("SD").copied().unwrap_or(0), "captures_total":candidates.len()*2},
            {"id":"CP", "label":"Check Performance", "status":"unavailable"}
        ],
        "phases": report.phases,
        "candidate_count": candidates.len(),
        "report": report,
    });
    let output = &args.output;
    let temporary = output.join("telemetry.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&document)?)?;
    fs::rename(temporary, output.join("telemetry.json"))?;
    if let Some(group_dir) = &args.observatory_group_dir {
        ensure!(
            group_dir.is_dir(),
            "Observatory group directory does not exist"
        );
        let staging = group_dir.join("targeted-comparison.json.tmp");
        fs::write(&staging, serde_json::to_vec_pretty(&document)?)?;
        fs::rename(staging, group_dir.join("targeted-comparison.json"))?;
    }
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

fn open_workbooks(excel: &mut ExcelHost, paths: &WorkingPaths) -> Result<BTreeMap<String, u32>> {
    let mut pids = BTreeMap::new();
    let original = excel.call(json!({"kind":"open", "lane":"O", "path":paths.original}))?;
    if let Some(pid) = original["pid"].as_u64() {
        pids.insert("O".into(), pid as u32);
    }
    let default = excel.call(json!({"kind":"open", "lane":"SD", "path":paths.default}))?;
    if let Some(pid) = default["pid"].as_u64() {
        pids.insert("SD".into(), pid as u32);
    }
    Ok(pids)
}

/// Adds cells that Excel changes during an ordinary save, before either rebuild.
fn save_and_extend_candidates(
    excel: &mut ExcelHost,
    paths: &WorkingPaths,
    candidates: &mut BTreeSet<Candidate>,
) -> Result<BTreeSet<Candidate>> {
    let before_original = paths.original.with_extension("before-save.xlsx");
    let before_default = paths.default.with_extension("before-save.xlsx");
    fs::copy(&paths.original, &before_original)?;
    fs::copy(&paths.default, &before_default)?;
    excel.call(json!({"kind":"save", "lane":"O"}))?;
    excel.call(json!({"kind":"save", "lane":"SD"}))?;
    let original_save_changes = discover_candidates(&before_original, &paths.original)?;
    candidates.extend(original_save_changes.iter().cloned());
    candidates.extend(discover_candidates(&before_default, &paths.default)?);
    ensure!(
        candidates.len() <= MAX_CANDIDATES,
        "more than {MAX_CANDIDATES} changed cells; aborting bounded experiment"
    );
    Ok(original_save_changes)
}

/// Maps changed cell XML to visible cell or row-geometry capture candidates.
/// The fixed candidate cap prevents accidental whole-sheet screenshot work.
fn discover_candidates(left: &Path, right: &Path) -> Result<BTreeSet<Candidate>> {
    let left_plan = build_workbook_visual_plan(left)?;
    let right_plan = build_workbook_visual_plan(right)?;
    let mut found = BTreeSet::new();
    for left_sheet in &left_plan.sheets {
        let Some(right_sheet) = right_plan
            .sheets
            .iter()
            .find(|sheet| sheet.sheet == left_sheet.sheet)
        else {
            bail!("sheet removed: {}", left_sheet.sheet);
        };
        let left_cells = read_cells(left, &left_sheet.sheet_file)?;
        let right_cells = read_cells(right, &right_sheet.sheet_file)?;
        for cell in left_cells.keys().chain(right_cells.keys()) {
            if left_cells.get(cell) != right_cells.get(cell) {
                let changed_cell = if has_visible_content(left_cells.get(cell))
                    || has_visible_content(right_cells.get(cell))
                {
                    cell.clone()
                } else {
                    // Empty styled cells can change row height. One geometry
                    // probe per row is enough even when Excel removes 16k cells.
                    let row =
                        cell.trim_start_matches(|character: char| character.is_ascii_alphabetic());
                    format!("A{row}")
                };
                found.insert(Candidate {
                    sheet: left_sheet.sheet.clone(),
                    cell: changed_cell,
                });
                ensure!(
                    found.len() <= MAX_CANDIDATES,
                    "more than {MAX_CANDIDATES} changed cells; aborting bounded experiment"
                );
            }
        }
    }
    Ok(found)
}

fn has_visible_content(cell: Option<&Vec<u8>>) -> bool {
    cell.is_some_and(|bytes| {
        [b"<v".as_slice(), b"<f", b"<is", b"<t"]
            .iter()
            .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
    })
}

/// Reads bounded worksheet fragments, then indexes cells for this WIP comparison.
/// The map still scales with the cell count; a streaming merge is needed for
/// large production sheets.
fn read_cells(path: &Path, sheet_file: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let sheet = archive.by_name(sheet_file)?;
    let mut stream = WorksheetStream::new(sheet);
    let mut fragment = Vec::new();
    let mut cells = BTreeMap::new();
    while stream.next(&mut fragment)? {
        let mut reader = Reader::<128>::new(&fragment)?;
        while let Some(tag) = reader.next()? {
            if tag.local() == b"c" && matches!(tag.kind, Kind::Start | Kind::Empty) {
                let Some(reference) = tag.attr(b"r")? else {
                    continue;
                };
                let address = String::from_utf8(reference.to_vec())?;
                let end = if tag.kind == Kind::Empty {
                    tag.end
                } else {
                    let mut end = tag.end;
                    while let Some(child) = reader.next()? {
                        end = child.end;
                        if child.local() == b"c" && child.kind == Kind::End {
                            break;
                        }
                    }
                    end
                };
                cells.insert(address, fragment[tag.start..end].to_vec());
            }
        }
    }
    Ok(cells)
}

struct Evidence {
    path: PathBuf,
    geometry: Value,
}

/// Saves a tiny picture and rendered geometry for every selected cell.
fn capture_candidates(
    excel: &mut ExcelHost,
    lane: &str,
    candidates: &BTreeSet<Candidate>,
    args: &Args,
    report: &mut Report,
    stage: &str,
) -> Result<BTreeMap<Candidate, Evidence>> {
    let directory = args.output.join(stage);
    fs::create_dir_all(&directory)?;
    let mut captured = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let range = padded_range(&candidate.cell)?;
        let path = directory.join(format!("{index:03}.png"));
        let geometry = excel.call(
            json!({"kind":"measure", "lane":lane, "sheet":candidate.sheet, "range":candidate.cell}),
        )?;
        excel.call(json!({"kind":"capture", "lane":lane, "sheet":candidate.sheet, "range":range, "output":path}))?;
        captured.insert(candidate.clone(), Evidence { path, geometry });
        *report.capture_counts.entry(lane.to_owned()).or_default() += 1;
        publish(args, report, &format!("capture:{lane}"), candidates)?;
    }
    Ok(captured)
}

fn padded_range(address: &str) -> Result<String> {
    let split = address
        .find(|c: char| c.is_ascii_digit())
        .context("invalid cell address")?;
    let (letters, digits) = address.split_at(split);
    let row: u32 = digits.parse()?;
    let column = letters
        .bytes()
        .try_fold(0u32, |value, digit| {
            value
                .checked_mul(26)?
                .checked_add(u32::from(digit.to_ascii_uppercase().checked_sub(b'A')? + 1))
        })
        .context("invalid column")?;
    ensure!(
        row > 0 && column > 0 && row <= 1_048_576 && column <= 16_384,
        "cell out of Excel bounds"
    );
    Ok(format!(
        "{}{}:{}{}",
        column_name(column.saturating_sub(1).max(1)),
        row.saturating_sub(1).max(1),
        column_name((column + 1).min(16_384)),
        (row + 1).min(1_048_576)
    ))
}

fn column_name(mut column: u32) -> String {
    let mut result = String::new();
    while column > 0 {
        column -= 1;
        result.insert(0, (b'A' + (column % 26) as u8) as char);
        column /= 26;
    }
    result
}

fn compare_evidence(
    candidates: &BTreeSet<Candidate>,
    original_before: &BTreeMap<Candidate, Evidence>,
    shrink_before: &BTreeMap<Candidate, Evidence>,
    shrink_after: &BTreeMap<Candidate, Evidence>,
    original_after: &BTreeMap<Candidate, Evidence>,
    original_save_changes: &BTreeSet<Candidate>,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for candidate in candidates {
        let o0 = &original_before[candidate];
        let s0 = &shrink_before[candidate];
        let s1 = &shrink_after[candidate];
        let o1 = &original_after[candidate];
        findings.push(Finding {
            sheet: candidate.sheet.clone(),
            cell: candidate.cell.clone(),
            range: padded_range(&candidate.cell)?,
            before_recalc_differs: images_differ(&o0.path, &s0.path)?,
            after_shrink_recalc_differs: images_differ(&o0.path, &s1.path)?,
            also_changed_by_original_recalc: images_differ(&o0.path, &o1.path)?
                || o0.geometry != o1.geometry,
            before_recalc_geometry_differs: o0.geometry != s0.geometry,
            after_shrink_recalc_geometry_differs: o0.geometry != s1.geometry,
            original_save_xml_changed: original_save_changes.contains(candidate),
            original_before_png: o0.path.clone(),
            shrink_before_png: s0.path.clone(),
            shrink_recalc_png: s1.path.clone(),
            original_recalc_png: o1.path.clone(),
            original_open_geometry: o0.geometry.clone(),
            shrink_open_geometry: s0.geometry.clone(),
            shrink_recalc_geometry: s1.geometry.clone(),
            original_recalc_geometry: o1.geometry.clone(),
        });
    }
    Ok(findings)
}

fn images_differ(left: &Path, right: &Path) -> Result<bool> {
    let left = image::open(left)?;
    let right = image::open(right)?;
    Ok(left.dimensions() != right.dimensions() || left.to_rgba8() != right.to_rgba8())
}

struct ExcelHost {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl ExcelHost {
    fn start(host_script: Option<&Path>) -> Result<Self> {
        let workspace_script =
            Path::new("excel-shrink-cli/office-capture/targeted-comparison-host.ps1");
        let package_script = Path::new("office-capture/targeted-comparison-host.ps1");
        let default_script = if workspace_script.exists() {
            workspace_script.to_path_buf()
        } else if package_script.exists() {
            package_script.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("office-capture/targeted-comparison-host.ps1")
        };
        let script = host_script.unwrap_or(&default_script);
        let mut child = Command::new("powershell.exe")
            .args(["-STA", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start persistent Excel host; Windows PowerShell is required")?;
        let input = BufWriter::new(child.stdin.take().context("Excel host stdin")?);
        let output = BufReader::new(child.stdout.take().context("Excel host stdout")?);
        Ok(Self {
            child,
            input,
            output,
        })
    }

    fn call(&mut self, command: Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, &command)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let mut line = String::new();
        ensure!(
            self.output.read_line(&mut line)? > 0,
            "Excel host exited without a response"
        );
        let response: Value = serde_json::from_str(&line).context("invalid Excel host response")?;
        ensure!(response["ok"] == true, "Excel host: {}", response["error"]);
        Ok(response["result"].clone())
    }
}

impl Drop for ExcelHost {
    fn drop(&mut self) {
        let _ = self.call(json!({"kind":"close", "lane":"SD"}));
        let _ = self.call(json!({"kind":"close", "lane":"O"}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_have_one_cell_margin_and_clip_at_edges() {
        assert_eq!(padded_range("A1").unwrap(), "A1:B2");
        assert_eq!(padded_range("E64").unwrap(), "D63:F65");
        assert_eq!(padded_range("XFD1048576").unwrap(), "XFC1048575:XFD1048576");
    }
}
