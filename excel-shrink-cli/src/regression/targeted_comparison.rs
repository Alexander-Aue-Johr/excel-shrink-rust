//! Standalone, targeted comparison of one original workbook and one shrink result.
//! The JSON report keeps skipped checks explicit; it is not a full visual proof.

mod candidates;
mod evidence;
mod host;

use anyhow::{ensure, Context, Result};
use candidates::discover_candidates;
use clap::Parser;
use evidence::{capture_candidates, compare_evidence};
use host::ExcelHost;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

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
