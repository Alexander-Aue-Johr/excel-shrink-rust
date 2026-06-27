use crate::pipeline::process_files;
use crate::Cli;
use clap::{Parser, Subcommand};
use excel_automation::{ExcelSavedWorkbook, ExcelScreenshotReport};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use xlsx_value_compare::{CellValueDifference, MissingCellPolicy, WorkbookComparisonReport};

mod excel_automation;
mod xlsx_value_compare;

#[derive(Parser, Debug)]
#[command(
    name = "excel-shrink regression",
    about = "Run real-file regression suites."
)]
pub struct RegressionCli {
    #[command(subcommand)]
    command: RegressionCommand,
}

#[derive(Subcommand, Debug)]
enum RegressionCommand {
    /// Run the developer-facing smoke regression suite.
    Smoke(RegressionRunArgs),
    /// Run the heavy corpus regression suite.
    Corpus(RegressionRunArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct RegressionRunArgs {
    /// Baseline JSON path. Defaults to the suite baseline under regression/baselines.
    #[arg(long)]
    baseline_path: Option<PathBuf>,

    /// Root containing the real XLSX corpus. Overrides EXCEL_SHRINK_CORPUS_ROOT.
    #[arg(long)]
    corpus_root: Option<PathBuf>,

    /// Directory for cleaned XLSX outputs.
    #[arg(long)]
    output_root: Option<PathBuf>,

    /// JSON report path.
    #[arg(long)]
    report_path: Option<PathBuf>,

    /// Run only one case by category ID.
    #[arg(long)]
    case_id: Option<String>,

    /// Allow a selected baseline to contain no cases.
    #[arg(long)]
    allow_empty: bool,

    /// Ignore the baseline-level defaults.cli_args for this run.
    #[arg(long)]
    ignore_default_cli_args: bool,

    /// Extra CLI argument passed to excel-shrink for each case. Repeat for multiple args.
    #[arg(long = "cli-arg", allow_hyphen_values = true)]
    cli_args: Vec<String>,

    /// Open the shrunk workbook with Excel, force formula recalculation, save it, and compare cached cell values.
    #[arg(long)]
    excel_recalc_check: bool,

    /// Open original and shrunk workbooks with Excel, screenshot used ranges, and save cropped differences.
    #[arg(long)]
    excel_screenshot_check: bool,
}

#[derive(Debug, Deserialize)]
struct RegressionSuite {
    defaults: Option<SuiteDefaults>,
    cases: Vec<RegressionCase>,
    aggregate: Option<AggregateExpectation>,
}

#[derive(Debug, Deserialize)]
struct SuiteDefaults {
    cli_args: Option<Vec<String>>,
    analysis_mode: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegressionCase {
    id: String,
    category: Option<String>,
    reason: Option<String>,
    dominant_cleanup: Option<Vec<String>>,
    notes: Option<String>,
    source_repository_url: Option<String>,
    source_file_url: Option<String>,
    input_path: Option<String>,
    relative_input: Option<String>,
    download_url: Option<String>,
    cached_file_name: Option<String>,
    cli_args: Option<Vec<String>>,
    analysis_mode: Option<bool>,
    max_output_bytes: Option<u64>,
    min_reduction_ratio: Option<f64>,
    analysis_checks: Option<Vec<AnalysisCheck>>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnalysisCheck {
    cause: String,
    sheet_file: Option<String>,
    min_removed_items: Option<f64>,
    min_removed_bytes_uncompressed_xml: Option<f64>,
    min_reduction_bytes_uncompressed_xml: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AggregateExpectation {
    max_total_output_bytes: Option<u64>,
    min_total_reduction_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct RegressionReport {
    mode: String,
    baseline_path: PathBuf,
    corpus_root: Option<PathBuf>,
    excel_recalc_check: bool,
    excel_screenshot_check: bool,
    output_root: PathBuf,
    created_at_unix_ms: u128,
    cases: Vec<CaseReport>,
    aggregate: Option<AggregateReport>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: String,
    category: Option<String>,
    reason: Option<String>,
    dominant_cleanup: Option<Vec<String>>,
    notes: Option<String>,
    source_repository_url: Option<String>,
    source_file_url: Option<String>,
    download_url: Option<String>,
    input_path: PathBuf,
    output_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_copy_path: Option<PathBuf>,
    original_bytes: u64,
    cleaned_bytes: Option<u64>,
    reduction_ratio: Option<f64>,
    analysis_mode: bool,
    failures: Vec<String>,
    top_sheet_reductions: Vec<SheetReductionDetail>,
    analysis_details: Vec<AnalysisDetail>,
    value_comparison_details: Option<WorkbookComparisonReport>,
    excel_recalc_details: Option<ExcelRecalcValidationReport>,
    excel_screenshot_details: Option<ExcelScreenshotReport>,
}

#[derive(Debug, Serialize)]
struct ExcelRecalcValidationReport {
    original_vs_shrunk: WorkbookComparisonReport,
    shrunk_recalculated: ExcelSavedWorkbook,
    shrunk_vs_recalculated: WorkbookComparisonReport,
    original_recalculated: Option<ExcelSavedWorkbook>,
    original_vs_recalculated_for_changed_sheets: Option<WorkbookComparisonReport>,
    volatile_difference_count: usize,
    real_recalc_differences: Vec<CellValueDifference>,
}

#[derive(Debug, Serialize)]
struct AnalysisDetail {
    cause: String,
    sheet_file: Option<String>,
    removed_items: f64,
    removed_bytes_uncompressed_xml: f64,
    reduction_bytes_uncompressed_xml: f64,
}

#[derive(Debug, Serialize)]
struct SheetReductionDetail {
    sheet_file: String,
    original_sheet_xml_bytes: u64,
    cleaned_sheet_xml_bytes: u64,
    reduction_bytes_uncompressed_xml: i64,
    reduction_ratio: f64,
}

#[derive(Debug, Serialize)]
struct AggregateReport {
    total_original_bytes: u64,
    total_cleaned_bytes: u64,
    total_reduction_ratio: f64,
    failures: Vec<String>,
}

static DOWNLOAD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn run_from_args<I, T>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = RegressionCli::parse_from(args);
    match cli.command {
        RegressionCommand::Smoke(args) => {
            run_suite("smoke", "regression/baselines/smoke.json", args)
        }
        RegressionCommand::Corpus(args) => {
            run_suite("corpus", "regression/baselines/corpus.json", args)
        }
    }
}

#[cfg(test)]
pub fn run_smoke_case_for_corpus_regression_test(case_id: &str) {
    init_corpus_regression_logging_for_test();

    let args = corpus_regression_run_args_for_test(Some(case_id.to_string()));
    run_suite("smoke", "regression/baselines/smoke.json", args)
        .unwrap_or_else(|e| panic!("corpus regression smoke case '{case_id}' failed: {e:#}"));
}

#[cfg(test)]
pub fn run_smoke_suite_for_corpus_regression_test() {
    init_corpus_regression_logging_for_test();

    let args = corpus_regression_run_args_for_test(None);
    run_suite("smoke", "regression/baselines/smoke.json", args)
        .unwrap_or_else(|e| panic!("corpus regression smoke suite failed: {e:#}"));
}

#[cfg(test)]
fn corpus_regression_run_args_for_test(case_id: Option<String>) -> RegressionRunArgs {
    let args = RegressionRunArgs {
        baseline_path: None,
        corpus_root: None,
        output_root: None,
        report_path: None,
        case_id,
        allow_empty: false,
        ignore_default_cli_args: env_flag_enabled_for_test(
            "EXCEL_SHRINK_CORPUS_REGRESSION_IGNORE_DEFAULT_CLI_ARGS",
        ),
        cli_args: corpus_regression_cli_args_from_env_for_test(),
        excel_recalc_check: env_flag_enabled_for_test(
            "EXCEL_SHRINK_CORPUS_REGRESSION_EXCEL_RECALC_CHECK",
        ),
        excel_screenshot_check: env_flag_enabled_for_test(
            "EXCEL_SHRINK_CORPUS_REGRESSION_EXCEL_SCREENSHOT_CHECK",
        ),
    };
    args
}

#[cfg(test)]
fn init_corpus_regression_logging_for_test() {
    static LOGGING: OnceLock<()> = OnceLock::new();

    LOGGING.get_or_init(|| {
        let level = std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_CONSOLE_LOG_LEVEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "info".to_string());

        let filter = tracing_subscriber::EnvFilter::new(level);
        let _ = tracing_subscriber::fmt()
            .compact()
            .with_target(false)
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .try_init();
    });
}

#[cfg(test)]
fn corpus_regression_cli_args_from_env_for_test() -> Vec<String> {
    let mut args = Vec::new();

    if let Ok(value) = std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_CHUNK_SIZE") {
        if !value.trim().is_empty() {
            args.push("--chunk-size".to_string());
            args.push(value.trim().to_string());
        }
    }

    if let Ok(value) = std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_PARALLEL_RANGE_CHUNK_SIZE") {
        if !value.trim().is_empty() {
            args.push("--parallel-range-chunk-size".to_string());
            args.push(value.trim().to_string());
        }
    }

    if let Ok(value) = std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_SHEETS") {
        let sheets: Vec<_> = value
            .split(',')
            .map(str::trim)
            .filter(|sheet| !sheet.is_empty())
            .collect();

        if !sheets.is_empty() {
            args.push("--sheets".to_string());
            args.extend(sheets.into_iter().map(str::to_string));
        }
    }

    args.extend(corpus_regression_cleanup_cli_args_from_env_for_test());

    args
}

#[cfg(test)]
fn corpus_regression_cleanup_cli_args_from_env_for_test() -> Vec<String> {
    let mut args = Vec::new();

    let keep_empty_style_sentinel_cells =
        match std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_KEEP_EMPTY_STYLE_SENTINEL_CELLS") {
            Ok(value) if !value.trim().is_empty() => env_flag_value_enabled_for_test(&value),
            _ => !env_flag_enabled_for_test(
                "EXCEL_SHRINK_CORPUS_REGRESSION_DISABLE_EMPTY_STYLE_REPRESENTATIVES",
            ),
        };

    if !keep_empty_style_sentinel_cells {
        args.push("--disable-empty-style-representatives".to_string());
    }

    if !env_flag_enabled_for_test(
        "EXCEL_SHRINK_CORPUS_REGRESSION_REMOVE_UNREFERENCED_WHITESPACE_CELLS",
    ) {
        args.push("--keep-unreferenced-whitespace-cells".to_string());
    }

    if env_flag_enabled_for_test(
        "EXCEL_SHRINK_CORPUS_REGRESSION_REMOVE_UNREFERENCED_HIDDEN_CONTENT_CELLS",
    ) {
        args.push("--remove-unreferenced-hidden-content-cells".to_string());
    }

    args
}

#[cfg(test)]
fn env_flag_enabled_for_test(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| env_flag_value_enabled_for_test(&value))
        .unwrap_or(false)
}

#[cfg(test)]
fn env_flag_value_enabled_for_test(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes")
}

#[cfg(test)]
pub fn available_smoke_cases_for_corpus_regression_tests() -> Vec<String> {
    let suite = load_suite(&repo_root().join("regression/baselines/smoke.json"));
    suite.cases.into_iter().map(|case| case.id).collect()
}

#[cfg(all(test, feature = "cell-tracing"))]
pub fn run_seeschifffahrt_sheet11_trace_removed_cells_for_corpus_regression_test(
    selected_cell: Option<&str>,
) {
    use excel_shrink_core::cleaning_trace::{self, CellDecision, CellVisitTarget};
    use excel_shrink_core::models::CleanupCause;

    init_corpus_regression_logging_for_test();
    cleaning_trace::clear();

    const SHEET_FILE: &str = "xl/worksheets/sheet11.xml";
    let expected_cells = [
        ("A1", 1_u32, b"1".as_slice()),
        ("F2", 6_u32, b"2".as_slice()),
        ("B22", 2_u32, b"22".as_slice()),
    ];

    let selected_cell = selected_cell
        .map(str::trim)
        .filter(|cell| !cell.is_empty() && !cell.eq_ignore_ascii_case("all"));
    let cells_to_check: Vec<_> = expected_cells
        .into_iter()
        .filter(|(cell_ref, _, _)| {
            selected_cell.map_or(true, |selected| cell_ref.eq_ignore_ascii_case(selected))
        })
        .collect();
    assert!(
        !cells_to_check.is_empty(),
        "unknown seeschifffahrt sheet11 trace cell {:?}; expected one of all, A1, F2, B22",
        selected_cell
    );
    cleaning_trace::set_cell_visit_targets(
        cells_to_check
            .iter()
            .map(|(_, col, row)| CellVisitTarget {
                sheet_file: SHEET_FILE.to_string(),
                col: *col,
                row: row.to_vec(),
            })
            .collect(),
    );

    let repo_root = repo_root();
    let suite = load_suite(&repo_root.join("regression/baselines/smoke.json"));
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == "trailing-layout-rows")
        .expect("smoke baseline should contain trailing-layout-rows case");
    let corpus_root = std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT")
        .map(PathBuf::from)
        .map(|path| resolve_path(&repo_root, path));
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case)
        .unwrap_or_else(|e| panic!("failed to resolve seeschifffahrt input: {e:#}"));
    let output_root = repo_root
        .join("regression/tmp")
        .join(format!("cell-trace-{}", timestamp()));
    fs::create_dir_all(&output_root)
        .unwrap_or_else(|e| panic!("failed to create cell trace output directory: {e}"));

    let mut cli_args = vec![
        "excel-shrink".to_string(),
        input.to_string_lossy().to_string(),
        output_root.to_string_lossy().to_string(),
        "--force-overwrite".to_string(),
        "--disable-multiprocessing".to_string(),
        "--writer-workers".to_string(),
        "1".to_string(),
    ];
    cli_args.extend(corpus_regression_cleanup_cli_args_from_env_for_test());
    cli_args.extend(["--sheets".to_string(), SHEET_FILE.to_string()]);
    let cli = Cli::parse_from(cli_args);

    process_files(vec![input], &cli)
        .unwrap_or_else(|e| panic!("seeschifffahrt sheet11 trace run failed: {e:#}"));

    let decisions = cleaning_trace::recorded_cell_decisions();
    let visits = cleaning_trace::recorded_cell_visits();
    for (cell_ref, col, row) in cells_to_check {
        assert!(
            visits.iter().any(|visit| {
                visit.sheet_file == SHEET_FILE && visit.col == col && visit.row == row
            }),
            "expected {SHEET_FILE}!{cell_ref} to be visited; matching trace event was not recorded"
        );
        assert!(
            decisions.iter().any(|decision| {
                decision.sheet_file == SHEET_FILE
                    && decision.col == col
                    && decision.row == row
                    && decision.decision == CellDecision::Removed
                    && decision.cause == CleanupCause::EmptyCellsWithoutContent
            }),
            "expected {SHEET_FILE}!{cell_ref} to be removed; matching trace event was not recorded"
        );
    }
}

#[cfg(all(test, feature = "cell-tracing"))]
pub fn run_kernhaushalt_sheet84_trace_removed_cell_for_corpus_regression_test() {
    use excel_shrink_core::cleaning_trace::{self, CellDecision, CellVisitTarget};
    use excel_shrink_core::models::CleanupCause;

    init_corpus_regression_logging_for_test();
    cleaning_trace::clear();

    const SHEET_FILE: &str = "xl/worksheets/sheet84.xml";
    const CELL_REF: &str = "D1048576";
    const COL: u32 = 4;
    const ROW: &[u8] = b"1048576";
    cleaning_trace::set_cell_visit_targets(vec![CellVisitTarget {
        sheet_file: SHEET_FILE.to_string(),
        col: COL,
        row: ROW.to_vec(),
    }]);

    let repo_root = repo_root();
    let suite = load_suite(&repo_root.join("regression/baselines/smoke.json"));
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == "overall-reduction")
        .expect("smoke baseline should contain overall-reduction case");
    let corpus_root = std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT")
        .map(PathBuf::from)
        .map(|path| resolve_path(&repo_root, path));
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case)
        .unwrap_or_else(|e| panic!("failed to resolve kernhaushalt input: {e:#}"));
    let output_root = repo_root
        .join("regression/tmp")
        .join(format!("cell-trace-{}", timestamp()));
    fs::create_dir_all(&output_root)
        .unwrap_or_else(|e| panic!("failed to create cell trace output directory: {e}"));

    let mut cli_args = vec![
        "excel-shrink".to_string(),
        input.to_string_lossy().to_string(),
        output_root.to_string_lossy().to_string(),
        "--force-overwrite".to_string(),
        "--disable-multiprocessing".to_string(),
        "--writer-workers".to_string(),
        "1".to_string(),
    ];
    cli_args.extend(corpus_regression_cleanup_cli_args_from_env_for_test());
    cli_args.extend([
        "--disable-tail-pruning".to_string(),
        "--parallel-range-chunk-size".to_string(),
        "0".to_string(),
        "--sheets".to_string(),
        SHEET_FILE.to_string(),
    ]);
    let cli = Cli::parse_from(cli_args);

    process_files(vec![input], &cli)
        .unwrap_or_else(|e| panic!("kernhaushalt sheet84 trace run failed: {e:#}"));

    let decisions = cleaning_trace::recorded_cell_decisions();
    let visits = cleaning_trace::recorded_cell_visits();
    assert!(
        visits.iter().any(|visit| {
            visit.sheet_file == SHEET_FILE && visit.col == COL && visit.row == ROW
        }),
        "expected {SHEET_FILE}!{CELL_REF} to be visited; matching trace event was not recorded"
    );
    assert!(
        decisions.iter().any(|decision| {
            decision.sheet_file == SHEET_FILE
                && decision.col == COL
                && decision.row == ROW
                && decision.decision == CellDecision::Removed
                && decision.cause == CleanupCause::EmptyCellsWithoutContent
        }),
        "expected {SHEET_FILE}!{CELL_REF} to be removed; matching trace event was not recorded"
    );
}

#[cfg(all(test, feature = "cell-tracing"))]
pub fn run_seeschifffahrt_sheet1_h15_trace_visited_cell_for_corpus_regression_test() {
    run_single_smoke_case_trace_cell_for_corpus_regression_test(
        "trailing-layout-rows",
        "xl/worksheets/sheet1.xml",
        "H15",
        8,
        b"15",
        "seeschifffahrt sheet1 H15",
        &[],
        false,
    );
}

#[cfg(all(test, feature = "cell-tracing"))]
pub fn run_foerderprogramme_sheet2_a5_trace_visited_cell_for_corpus_regression_test() {
    run_single_smoke_case_trace_cell_for_corpus_regression_test(
        "column-range-merge",
        "xl/worksheets/sheet2.xml",
        "A5",
        1,
        b"5",
        "foerderprogramme sheet2 A5",
        &[],
        false,
    );
}

#[cfg(all(test, feature = "cell-tracing"))]
fn run_single_smoke_case_trace_cell_for_corpus_regression_test(
    case_id: &str,
    sheet_file: &str,
    cell_ref: &str,
    col: u32,
    row: &[u8],
    label: &str,
    extra_cli_args: &[&str],
    expect_removed: bool,
) {
    use excel_shrink_core::cleaning_trace::{self, CellDecision, CellVisitTarget};
    use excel_shrink_core::models::CleanupCause;

    init_corpus_regression_logging_for_test();
    cleaning_trace::clear();
    cleaning_trace::set_cell_visit_targets(vec![CellVisitTarget {
        sheet_file: sheet_file.to_string(),
        col,
        row: row.to_vec(),
    }]);

    let repo_root = repo_root();
    let suite = load_suite(&repo_root.join("regression/baselines/smoke.json"));
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .unwrap_or_else(|| panic!("smoke baseline should contain {case_id} case"));
    let corpus_root = std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT")
        .map(PathBuf::from)
        .map(|path| resolve_path(&repo_root, path));
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case)
        .unwrap_or_else(|e| panic!("failed to resolve {label} input: {e:#}"));
    let output_root = repo_root
        .join("regression/tmp")
        .join(format!("cell-trace-{}", timestamp()));
    fs::create_dir_all(&output_root)
        .unwrap_or_else(|e| panic!("failed to create cell trace output directory: {e}"));

    let mut cli_args = vec![
        "excel-shrink".to_string(),
        input.to_string_lossy().to_string(),
        output_root.to_string_lossy().to_string(),
        "--force-overwrite".to_string(),
        "--disable-multiprocessing".to_string(),
        "--writer-workers".to_string(),
        "1".to_string(),
    ];
    cli_args.extend(corpus_regression_cleanup_cli_args_from_env_for_test());
    cli_args.extend(extra_cli_args.iter().map(|arg| (*arg).to_string()));
    cli_args.extend(["--sheets".to_string(), sheet_file.to_string()]);
    let cli = Cli::parse_from(cli_args);

    process_files(vec![input], &cli).unwrap_or_else(|e| panic!("{label} trace run failed: {e:#}"));

    let decisions = cleaning_trace::recorded_cell_decisions();
    let visits = cleaning_trace::recorded_cell_visits();
    assert!(
        visits
            .iter()
            .any(|visit| visit.sheet_file == sheet_file && visit.col == col && visit.row == row),
        "expected {sheet_file}!{cell_ref} to be visited; matching trace event was not recorded"
    );
    if expect_removed {
        assert!(
            decisions.iter().any(|decision| {
                decision.sheet_file == sheet_file
                    && decision.col == col
                    && decision.row == row
                    && decision.decision == CellDecision::Removed
                    && decision.cause == CleanupCause::EmptyCellsWithoutContent
            }),
            "expected {sheet_file}!{cell_ref} to be removed; matching trace event was not recorded"
        );
    }
}

fn run_suite(
    mode_name: &str,
    default_baseline_relative_path: &str,
    args: RegressionRunArgs,
) -> anyhow::Result<()> {
    let repo_root = repo_root();
    let baseline_path = resolve_path(
        &repo_root,
        args.baseline_path
            .unwrap_or_else(|| PathBuf::from(default_baseline_relative_path)),
    );
    let suite = load_suite(&baseline_path);
    let mut cases = suite.cases.clone();

    if let Some(case_id) = &args.case_id {
        cases.retain(|case| case.id == *case_id);
        anyhow::ensure!(
            !cases.is_empty(),
            "case id '{}' not found in baseline '{}'",
            case_id,
            baseline_path.display()
        );
    }

    anyhow::ensure!(
        args.allow_empty || !cases.is_empty(),
        "Baseline file contains no cases: {}",
        baseline_path.display()
    );

    let timestamp = timestamp();
    let output_root = args
        .output_root
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| {
            repo_root
                .join("regression/tmp")
                .join(format!("{mode_name}-{timestamp}"))
        });
    fs::create_dir_all(&output_root)?;

    let report_path = args
        .report_path
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| {
            repo_root
                .join("regression/reports")
                .join(format!("{mode_name}-{timestamp}.json"))
        });
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let corpus_root = args
        .corpus_root
        .or_else(|| std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT").map(PathBuf::from))
        .map(|path| resolve_path(&repo_root, path));

    let mut reports = Vec::new();
    let mut any_failure = false;
    let suite_defaults = if args.ignore_default_cli_args {
        None
    } else {
        suite.defaults.as_ref()
    };

    for case in cases {
        eprintln!("[{mode_name}] Running case '{}'", case.id);
        let report = run_case(
            mode_name,
            &repo_root,
            &output_root,
            corpus_root.as_deref(),
            suite_defaults,
            &args.cli_args,
            args.excel_recalc_check,
            args.excel_screenshot_check,
            &case,
        );
        match &report {
            Ok(report) if report.failures.is_empty() => {
                let ratio_text = report
                    .reduction_ratio
                    .map(|ratio| format!("{:.2}%", ratio * 100.0))
                    .unwrap_or_else(|| "n/a".to_string());
                eprintln!(
                    "[{mode_name}] PASS {} (output={} bytes, reduction={})",
                    report.id,
                    report.cleaned_bytes.unwrap_or(0),
                    ratio_text
                );
                if !report.top_sheet_reductions.is_empty() {
                    eprintln!(
                        "[{mode_name}]   Top sheet reductions: {}",
                        summarize_top_sheet_reductions(&report.top_sheet_reductions)
                    );
                }
            }
            Ok(report) => {
                any_failure = true;
                eprintln!("[{mode_name}] FAIL {}", report.id);
                for failure in &report.failures {
                    eprintln!("  - {failure}");
                }
                if !report.top_sheet_reductions.is_empty() {
                    eprintln!(
                        "[{mode_name}]   Top sheet reductions: {}",
                        summarize_top_sheet_reductions(&report.top_sheet_reductions)
                    );
                }
            }
            Err(error) => {
                any_failure = true;
                eprintln!("[{mode_name}] FAIL {}", case.id);
                eprintln!("  - {error:#}");
                reports.push(failed_case_report(
                    &repo_root,
                    &output_root,
                    &case,
                    error.to_string(),
                ));
                continue;
            }
        }
        reports.push(report?);
    }

    let aggregate = suite
        .aggregate
        .as_ref()
        .map(|expectation| aggregate_report(&reports, expectation));
    if let Some(aggregate) = &aggregate {
        if aggregate.failures.is_empty() {
            eprintln!(
                "[{mode_name}] Aggregate: output={} bytes, reduction={:.2}%",
                aggregate.total_cleaned_bytes,
                aggregate.total_reduction_ratio * 100.0
            );
        } else {
            any_failure = true;
            eprintln!("[{mode_name}] Aggregate failed");
            for failure in &aggregate.failures {
                eprintln!("  - {failure}");
            }
        }
    }

    let report = RegressionReport {
        mode: mode_name.to_string(),
        baseline_path,
        corpus_root,
        excel_recalc_check: args.excel_recalc_check,
        excel_screenshot_check: args.excel_screenshot_check,
        output_root,
        created_at_unix_ms: unix_ms(),
        cases: reports,
        aggregate,
    };
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    eprintln!("[{mode_name}] Report written to {}", report_path.display());

    anyhow::ensure!(!any_failure, "One or more regression checks failed.");
    Ok(())
}

fn run_case(
    _mode_name: &str,
    repo_root: &Path,
    output_root: &Path,
    corpus_root: Option<&Path>,
    defaults: Option<&SuiteDefaults>,
    extra_cli_args: &[String],
    excel_recalc_check: bool,
    excel_screenshot_check: bool,
    case: &RegressionCase,
) -> anyhow::Result<CaseReport> {
    let input = resolve_case_input(repo_root, corpus_root, case)?;
    let case_output_dir = output_root.join(&case.id);
    fs::create_dir_all(&case_output_dir)?;
    let analysis_mode = analysis_mode(defaults, case);
    let analysis_csv_path = case_output_dir.join("analysis.csv");
    let cli = build_cli(
        &input,
        &case_output_dir,
        defaults,
        case,
        analysis_mode,
        &analysis_csv_path,
        extra_cli_args,
    );

    let mut failures = Vec::new();
    if let Err(error) = process_files(vec![input.clone()], &cli) {
        failures.push(format!("excel-shrink failed: {error:#}"));
    }

    let output_path = case_output_dir.join(
        input
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("input file has no file name: {}", input.display()))?,
    );
    let original_bytes = fs::metadata(&input)?.len();
    let mut cleaned_bytes = None;
    let mut reduction_ratio = None;
    let mut top_sheet_reductions = Vec::new();
    let mut analysis_details = Vec::new();
    let mut value_comparison_details = None;
    let mut excel_recalc_details = None;
    let mut excel_screenshot_details = None;

    if output_path.exists() {
        let output_bytes = fs::metadata(&output_path)?.len();
        let ratio = if original_bytes == 0 {
            0.0
        } else {
            (original_bytes as f64 - output_bytes as f64) / original_bytes as f64
        };
        cleaned_bytes = Some(output_bytes);
        reduction_ratio = Some(ratio);
        match top_sheet_reductions_by_uncompressed_xml(&input, &output_path, 5) {
            Ok(reductions) => top_sheet_reductions = reductions,
            Err(error) => failures.push(format!("sheet reduction analysis failed: {error:#}")),
        }

        if let Some(max_output_bytes) = case.max_output_bytes {
            if output_bytes > max_output_bytes {
                failures.push(format!(
                    "output bytes {output_bytes} exceed max_output_bytes {max_output_bytes}"
                ));
            }
        }
        if let Some(min_reduction_ratio) = case.min_reduction_ratio {
            if ratio < min_reduction_ratio {
                failures.push(format!(
                    "reduction ratio {ratio} is below min_reduction_ratio {min_reduction_ratio}"
                ));
            }
        }
    } else {
        failures.push(format!(
            "expected output file not found: {}",
            output_path.display()
        ));
    }

    if analysis_mode {
        if !analysis_csv_path.exists() {
            failures.push(format!(
                "analysis CSV not found: {}",
                analysis_csv_path.display()
            ));
        } else if let Some(checks) = &case.analysis_checks {
            let (check_failures, details) = test_analysis_checks(&analysis_csv_path, checks)?;
            failures.extend(check_failures);
            analysis_details = details;
        }
    }

    if output_path.exists() && excel_recalc_check {
        match validate_excel_recalculation(&input, &output_path, &case_output_dir, &cli) {
            Ok((report, validation_failures)) => {
                value_comparison_details = Some(report.original_vs_shrunk.clone());
                failures.extend(validation_failures);
                excel_recalc_details = Some(report);
            }
            Err(error) => {
                failures.push(format!("Excel recalculation validation failed: {error:#}"))
            }
        }
    }

    if output_path.exists() && excel_screenshot_check {
        if value_comparison_details.is_none() {
            match compare_original_and_shrunk_values(&input, &output_path, &cli) {
                Ok(report) => {
                    if !report.differences.is_empty() {
                        failures.push(value_comparison_failure(&report));
                    }
                    value_comparison_details = Some(report);
                }
                Err(error) => {
                    failures.push(format!("cell value comparison failed: {error:#}"));
                }
            }
        }

        let screenshot_dir = case_output_dir.join("excel-screenshots");
        let work_dir = case_output_dir.join("excel-automation");
        let range_limits = value_comparison_details
            .as_ref()
            .map(|report| report.right_capture_ranges.clone())
            .unwrap_or_default();
        match excel_automation::compare_sheet_screenshots(
            &input,
            &output_path,
            &screenshot_dir,
            &work_dir,
            &range_limits,
        ) {
            Ok(report) => {
                if report.differences_found {
                    failures.push(format!(
                        "Excel screenshot comparison found differences; report: {}",
                        report.report_path.display()
                    ));
                }
                excel_screenshot_details = Some(report);
            }
            Err(error) => failures.push(format!("Excel screenshot validation failed: {error:#}")),
        }
    }

    let mut original_copy_path = None;
    if !failures.is_empty() {
        match copy_original_input_for_failed_case(&input, &case_output_dir) {
            Ok(path) => original_copy_path = Some(path),
            Err(error) => failures.push(format!(
                "failed to copy original workbook into case output directory: {error:#}"
            )),
        }
    }

    Ok(CaseReport {
        id: case.id.clone(),
        category: case.category.clone(),
        reason: case.reason.clone(),
        dominant_cleanup: case.dominant_cleanup.clone(),
        notes: case.notes.clone(),
        source_repository_url: case.source_repository_url.clone(),
        source_file_url: case.source_file_url.clone(),
        download_url: case.download_url.clone(),
        input_path: input,
        output_path,
        original_copy_path,
        original_bytes,
        cleaned_bytes,
        reduction_ratio,
        analysis_mode,
        failures,
        top_sheet_reductions,
        analysis_details,
        value_comparison_details,
        excel_recalc_details,
        excel_screenshot_details,
    })
}

fn copy_original_input_for_failed_case(
    input: &Path,
    case_output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let file_name = file_name(input)?;
    let destination = case_output_dir.join(format!("original-{}", file_name.to_string_lossy()));
    fs::copy(input, &destination).map_err(|error| {
        anyhow::anyhow!(
            "failed to copy '{}' to '{}': {error}",
            input.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

fn compare_original_and_shrunk_values(
    original_path: &Path,
    shrunk_path: &Path,
    cli: &Cli,
) -> anyhow::Result<WorkbookComparisonReport> {
    xlsx_value_compare::compare_workbook_values(
        original_path,
        shrunk_path,
        missing_cell_policy(cli),
        None,
    )
}

fn validate_excel_recalculation(
    original_path: &Path,
    shrunk_path: &Path,
    case_output_dir: &Path,
    cli: &Cli,
) -> anyhow::Result<(ExcelRecalcValidationReport, Vec<String>)> {
    let excel_dir = case_output_dir.join("excel-recalc");
    let automation_dir = case_output_dir.join("excel-automation");
    fs::create_dir_all(&excel_dir)?;

    let original_vs_shrunk = compare_original_and_shrunk_values(original_path, shrunk_path, cli)?;

    let shrunk_recalculated_path = excel_dir
        .join("shrunk-recalculated")
        .join(file_name(shrunk_path)?);
    let shrunk_recalculated = excel_automation::save_recalculated_copy(
        shrunk_path,
        &shrunk_recalculated_path,
        &automation_dir,
    )?;

    let shrunk_vs_recalculated = xlsx_value_compare::compare_workbook_values(
        shrunk_path,
        &shrunk_recalculated.path,
        MissingCellPolicy::strict(),
        None,
    )?;

    let mut failures = Vec::new();
    if !original_vs_shrunk.differences.is_empty() {
        failures.push(value_comparison_failure(&original_vs_shrunk));
    }

    let mut original_recalculated = None;
    let mut original_vs_recalculated_for_changed_sheets = None;
    let mut volatile_difference_count = 0;
    let mut real_recalc_differences = Vec::new();

    if !shrunk_vs_recalculated.differences.is_empty() {
        let changed_sheets = sheets_with_differences(&shrunk_vs_recalculated.differences);
        let original_recalculated_path = excel_dir
            .join("original-recalculated")
            .join(excel_saved_file_name(original_path)?);
        let saved_original = excel_automation::save_recalculated_copy(
            original_path,
            &original_recalculated_path,
            &automation_dir,
        )?;
        let original_recalc_report = xlsx_value_compare::compare_workbook_values(
            original_path,
            &saved_original.path,
            MissingCellPolicy::strict(),
            Some(&changed_sheets),
        )?;
        let volatile_keys =
            xlsx_value_compare::difference_keys(&original_recalc_report.differences);

        for difference in &shrunk_vs_recalculated.differences {
            let key = (difference.sheet_file.clone(), difference.cell_ref.clone());
            if volatile_keys.contains(&key) {
                volatile_difference_count += 1;
            } else {
                real_recalc_differences.push(difference.clone());
            }
        }

        if !real_recalc_differences.is_empty() {
            failures.push(format!(
                "shrunk workbook changed after Excel recalculation in {} non-volatile cells: {}",
                real_recalc_differences.len(),
                summarize_cell_differences(&real_recalc_differences)
            ));
        }

        original_recalculated = Some(saved_original);
        original_vs_recalculated_for_changed_sheets = Some(original_recalc_report);
    }

    Ok((
        ExcelRecalcValidationReport {
            original_vs_shrunk,
            shrunk_recalculated,
            shrunk_vs_recalculated,
            original_recalculated,
            original_vs_recalculated_for_changed_sheets,
            volatile_difference_count,
            real_recalc_differences,
        },
        failures,
    ))
}

fn missing_cell_policy(cli: &Cli) -> MissingCellPolicy {
    MissingCellPolicy {
        allow_whitespace_values: !cli.keep_unreferenced_whitespace_cells,
        allow_hidden_rows_or_columns: cli.remove_unreferenced_hidden_content_cells,
    }
}

fn value_comparison_failure(report: &WorkbookComparisonReport) -> String {
    format!(
        "original-vs-shrunk cell value comparison found {} differences: {}",
        report.difference_count,
        summarize_cell_differences(&report.differences)
    )
}

fn sheets_with_differences(differences: &[CellValueDifference]) -> HashSet<String> {
    differences
        .iter()
        .map(|difference| difference.sheet_file.clone())
        .collect()
}

fn summarize_cell_differences(differences: &[CellValueDifference]) -> String {
    const LIMIT: usize = 8;
    let mut parts = differences
        .iter()
        .take(LIMIT)
        .map(|difference| {
            format!(
                "{}!{} ({:?})",
                difference.sheet_file, difference.cell_ref, difference.kind
            )
        })
        .collect::<Vec<_>>();
    if differences.len() > LIMIT {
        parts.push(format!("... {} more", differences.len() - LIMIT));
    }
    parts.join(", ")
}

fn file_name(path: &Path) -> anyhow::Result<&std::ffi::OsStr> {
    path.file_name()
        .ok_or_else(|| anyhow::anyhow!("path has no file name: {}", path.display()))
}

fn excel_saved_file_name(path: &Path) -> anyhow::Result<String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow::anyhow!("path has no UTF-8 file stem: {}", path.display()))?;
    Ok(format!("{stem}.excel-saved.xlsx"))
}

fn failed_case_report(
    repo_root: &Path,
    output_root: &Path,
    case: &RegressionCase,
    failure: String,
) -> CaseReport {
    CaseReport {
        id: case.id.clone(),
        category: case.category.clone(),
        reason: case.reason.clone(),
        dominant_cleanup: case.dominant_cleanup.clone(),
        notes: case.notes.clone(),
        source_repository_url: case.source_repository_url.clone(),
        source_file_url: case.source_file_url.clone(),
        download_url: case.download_url.clone(),
        input_path: repo_root.join(case.relative_input.as_deref().unwrap_or("")),
        output_path: output_root.join(&case.id),
        original_copy_path: None,
        original_bytes: 0,
        cleaned_bytes: None,
        reduction_ratio: None,
        analysis_mode: false,
        failures: vec![failure],
        top_sheet_reductions: Vec::new(),
        analysis_details: Vec::new(),
        value_comparison_details: None,
        excel_recalc_details: None,
        excel_screenshot_details: None,
    }
}

fn top_sheet_reductions_by_uncompressed_xml(
    original_path: &Path,
    cleaned_path: &Path,
    limit: usize,
) -> anyhow::Result<Vec<SheetReductionDetail>> {
    let original_sheets = worksheet_xml_sizes(original_path)?;
    let cleaned_sheets = worksheet_xml_sizes(cleaned_path)?;

    let mut reductions = Vec::new();
    for (sheet_file, original_size) in original_sheets {
        let cleaned_size = cleaned_sheets.get(&sheet_file).copied().unwrap_or(0);
        let reduction_bytes = original_size as i64 - cleaned_size as i64;
        if reduction_bytes <= 0 {
            continue;
        }

        reductions.push(SheetReductionDetail {
            sheet_file,
            original_sheet_xml_bytes: original_size,
            cleaned_sheet_xml_bytes: cleaned_size,
            reduction_bytes_uncompressed_xml: reduction_bytes,
            reduction_ratio: if original_size == 0 {
                0.0
            } else {
                reduction_bytes as f64 / original_size as f64
            },
        });
    }

    reductions.sort_by(|left, right| {
        right
            .reduction_bytes_uncompressed_xml
            .cmp(&left.reduction_bytes_uncompressed_xml)
            .then_with(|| left.sheet_file.cmp(&right.sheet_file))
    });
    reductions.truncate(limit);
    Ok(reductions)
}

fn worksheet_xml_sizes(path: &Path) -> anyhow::Result<HashMap<String, u64>> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut sizes = HashMap::new();

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if !name.starts_with("xl/worksheets/") || !name.ends_with(".xml") {
            continue;
        }

        let mut bytes_read = 0_u64;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            bytes_read += read as u64;
        }
        sizes.insert(name, bytes_read);
    }

    Ok(sizes)
}

fn summarize_top_sheet_reductions(reductions: &[SheetReductionDetail]) -> String {
    reductions
        .iter()
        .map(|detail| {
            format!(
                "{} {:.2}% ({} bytes)",
                detail.sheet_file,
                detail.reduction_ratio * 100.0,
                detail.reduction_bytes_uncompressed_xml
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn aggregate_report(reports: &[CaseReport], expectation: &AggregateExpectation) -> AggregateReport {
    let successful: Vec<_> = reports
        .iter()
        .filter(|report| report.failures.is_empty() && report.cleaned_bytes.is_some())
        .collect();
    let total_original_bytes = successful
        .iter()
        .map(|report| report.original_bytes)
        .sum::<u64>();
    let total_cleaned_bytes = successful
        .iter()
        .filter_map(|report| report.cleaned_bytes)
        .sum::<u64>();
    let total_reduction_ratio = if total_original_bytes == 0 {
        0.0
    } else {
        (total_original_bytes as f64 - total_cleaned_bytes as f64) / total_original_bytes as f64
    };
    let mut failures = Vec::new();

    if successful.len() != reports.len() {
        failures.push(
            "aggregate expectations were not evaluated against a fully successful run".to_string(),
        );
    }
    if let Some(max_total_output_bytes) = expectation.max_total_output_bytes {
        if total_cleaned_bytes > max_total_output_bytes {
            failures.push(format!(
                "total output bytes {total_cleaned_bytes} exceed max_total_output_bytes {max_total_output_bytes}"
            ));
        }
    }
    if let Some(min_total_reduction_ratio) = expectation.min_total_reduction_ratio {
        if total_reduction_ratio < min_total_reduction_ratio {
            failures.push(format!(
                "total reduction ratio {total_reduction_ratio} is below min_total_reduction_ratio {min_total_reduction_ratio}"
            ));
        }
    }

    AggregateReport {
        total_original_bytes,
        total_cleaned_bytes,
        total_reduction_ratio,
        failures,
    }
}

fn build_cli(
    input: &Path,
    output_dir: &Path,
    defaults: Option<&SuiteDefaults>,
    case: &RegressionCase,
    analysis_mode: bool,
    analysis_csv_path: &Path,
    extra_cli_args: &[String],
) -> Cli {
    let mut args = vec![
        "excel-shrink".to_string(),
        input.to_string_lossy().to_string(),
        output_dir.to_string_lossy().to_string(),
        "--force-overwrite".to_string(),
    ];
    if analysis_mode {
        args.push("--analysis-csv".to_string());
        args.push(analysis_csv_path.to_string_lossy().to_string());
    }
    if let Some(defaults) = defaults {
        if let Some(default_args) = &defaults.cli_args {
            args.extend(default_args.iter().cloned());
        }
    }
    if let Some(case_args) = &case.cli_args {
        args.extend(case_args.iter().cloned());
    }
    args.extend(extra_cli_args.iter().cloned());
    Cli::parse_from(args)
}

fn analysis_mode(defaults: Option<&SuiteDefaults>, case: &RegressionCase) -> bool {
    case.analysis_mode
        .or_else(|| defaults.and_then(|defaults| defaults.analysis_mode))
        .unwrap_or(false)
        || case
            .analysis_checks
            .as_ref()
            .is_some_and(|checks| !checks.is_empty())
}

fn resolve_case_input(
    repo_root: &Path,
    corpus_root: Option<&Path>,
    case: &RegressionCase,
) -> anyhow::Result<PathBuf> {
    if let Some(input_path) = &case.input_path {
        let path = normalize_cli_path(PathBuf::from(input_path));
        anyhow::ensure!(
            path.exists(),
            "Input file for case '{}' not found: {}",
            case.id,
            path.display()
        );
        return Ok(path);
    }

    if let (Some(corpus_root), Some(relative_input)) = (corpus_root, &case.relative_input) {
        let path = corpus_root.join(relative_input);
        if path.exists() {
            return Ok(path);
        }

        eprintln!(
            "[smoke] Input file for case '{}' not found under corpus root: {}",
            case.id,
            path.display()
        );
        eprintln!("[smoke] Falling back to local cache, sibling corpus, or download URL.");
    }

    if let Some(relative_input) = &case.relative_input {
        let cached_path = cached_corpus_root(repo_root).join(relative_input);
        if cached_path.exists() {
            return Ok(cached_path);
        }

        let sibling_path = sibling_corpus_root(repo_root).join(relative_input);
        if sibling_path.exists() {
            return Ok(sibling_path);
        }
    }

    ensure_downloaded_case_input(repo_root, case)
}

fn ensure_downloaded_case_input(
    repo_root: &Path,
    case: &RegressionCase,
) -> anyhow::Result<PathBuf> {
    let cached_root = cached_corpus_root(repo_root);
    fs::create_dir_all(&cached_root)?;
    let cached_path = cached_root.join(case_cached_file_name(case)?);
    if cached_path.exists() {
        return Ok(cached_path);
    }

    let download_url = case.download_url.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Case '{}' needs input_path, relative_input with corpus root, or download_url",
            case.id
        )
    })?;

    let _guard = DOWNLOAD_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    if cached_path.exists() {
        return Ok(cached_path);
    }

    eprintln!("[download] {download_url}");
    let status = Command::new("curl")
        .args(["-L", "--fail", "--output"])
        .arg(&cached_path)
        .arg(download_url)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to start curl download command: {e}"))?;
    anyhow::ensure!(
        status.success(),
        "curl download failed for '{}' -> '{}'",
        download_url,
        cached_path.display()
    );
    Ok(cached_path)
}

fn case_cached_file_name(case: &RegressionCase) -> anyhow::Result<String> {
    if let Some(name) = &case.cached_file_name {
        return Ok(name.clone());
    }
    for value in [&case.relative_input, &case.input_path] {
        if let Some(value) = value {
            if let Some(name) = Path::new(value).file_name().and_then(|name| name.to_str()) {
                return Ok(name.to_string());
            }
        }
    }
    if let Some(download_url) = &case.download_url {
        if let Some(name) = download_url.rsplit('/').next() {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!(
        "Case '{}' does not provide a resolvable file name.",
        case.id
    )
}

fn test_analysis_checks(
    analysis_csv_path: &Path,
    checks: &[AnalysisCheck],
) -> anyhow::Result<(Vec<String>, Vec<AnalysisDetail>)> {
    let rows = read_analysis_csv(analysis_csv_path)?;
    let mut failures = Vec::new();
    let mut details = Vec::new();

    for check in checks {
        let matching_rows = rows.iter().filter(|row| {
            row.get("cause") == Some(&check.cause)
                && check
                    .sheet_file
                    .as_ref()
                    .map_or(true, |sheet| row.get("sheet_file") == Some(sheet))
        });
        let mut detail = AnalysisDetail {
            cause: check.cause.clone(),
            sheet_file: check.sheet_file.clone(),
            removed_items: 0.0,
            removed_bytes_uncompressed_xml: 0.0,
            reduction_bytes_uncompressed_xml: 0.0,
        };

        for row in matching_rows {
            detail.removed_items += csv_number(row, "removed_items");
            detail.removed_bytes_uncompressed_xml +=
                csv_number(row, "removed_bytes_uncompressed_xml");
            detail.reduction_bytes_uncompressed_xml +=
                csv_number(row, "reduction_bytes_uncompressed_xml");
        }

        if let Some(expected) = check.min_removed_items {
            if detail.removed_items < expected {
                failures.push(format!(
                    "analysis check failed for cause '{}': removed_items {} < {}",
                    check.cause, detail.removed_items, expected
                ));
            }
        }
        if let Some(expected) = check.min_removed_bytes_uncompressed_xml {
            if detail.removed_bytes_uncompressed_xml < expected {
                failures.push(format!(
                    "analysis check failed for cause '{}': removed_bytes_uncompressed_xml {} < {}",
                    check.cause, detail.removed_bytes_uncompressed_xml, expected
                ));
            }
        }
        if let Some(expected) = check.min_reduction_bytes_uncompressed_xml {
            if detail.reduction_bytes_uncompressed_xml < expected {
                failures.push(format!(
                    "analysis check failed for cause '{}': reduction_bytes_uncompressed_xml {} < {}",
                    check.cause, detail.reduction_bytes_uncompressed_xml, expected
                ));
            }
        }
        details.push(detail);
    }

    Ok((failures, details))
}

fn read_analysis_csv(path: &Path) -> anyhow::Result<Vec<HashMap<String, String>>> {
    let raw = fs::read_to_string(path)?;
    let mut lines = raw.lines();
    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let headers = parse_csv_line(header_line);
    let mut rows = Vec::new();

    for line in lines {
        let values = parse_csv_line(line);
        let mut row = HashMap::new();
        for (index, header) in headers.iter().enumerate() {
            row.insert(
                header.clone(),
                values.get(index).cloned().unwrap_or_default(),
            );
        }
        rows.push(row);
    }
    Ok(rows)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field);
                field = String::new();
            }
            _ => field.push(ch),
        }
    }
    fields.push(field);
    fields
}

fn csv_number(row: &HashMap<String, String>, key: &str) -> f64 {
    row.get(key)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn load_suite(path: &Path) -> RegressionSuite {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read regression suite '{}': {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse regression suite '{}': {e}", path.display()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}

fn cached_corpus_root(repo_root: &Path) -> PathBuf {
    repo_root.join("corpus-local/original_destatis_xlsx_files")
}

fn sibling_corpus_root(repo_root: &Path) -> PathBuf {
    repo_root
        .parent()
        .expect("workspace parent should exist")
        .join("original_destatis_xlsx_files")
}

fn resolve_path(repo_root: &Path, path: PathBuf) -> PathBuf {
    let path = normalize_cli_path(path);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn normalize_cli_path(path: PathBuf) -> PathBuf {
    if cfg!(unix) {
        let text = path.to_string_lossy();
        if text.contains('\\') {
            return PathBuf::from(text.replace('\\', "/"));
        }
    }

    path
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn normalize_cli_path_accepts_backslash_separators_on_unix() {
        let path = normalize_cli_path(PathBuf::from("/repo\\..\\original_destatis_xlsx_files"));

        assert_eq!(path, PathBuf::from("/repo/../original_destatis_xlsx_files"));
    }
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_secs();
    seconds.to_string()
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_millis()
}
