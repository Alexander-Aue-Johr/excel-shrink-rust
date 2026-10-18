use crate::pipeline::process_files;
use crate::visual_profile::{
    predefined_visual_profiles, profile_artifact_root, write_visual_profile_metadata,
    ResolvedVisualProfile, VisualProfileConfig,
};
use crate::{Cli, LogLevel};
use clap::{Parser, Subcommand};
use excel_automation::{
    ExcelSavedWorkbook, ExcelScreenshotDifferenceSummary, WorkbookOpenTiming,
};
use visual_validation::ExcelScreenshotReport;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use xlsx_value_compare::{CellValueDifference, MissingCellPolicy, WorkbookComparisonReport};

mod visual_diagnostics;
mod excel_automation;
mod xlsx_value_compare;
mod artifact_cache;
mod xml_views;
mod office_plan;
mod row_heights;
mod office_capture;
mod visual_compare;
mod visual_report;
mod visual_validation;

#[derive(Parser, Debug)]
#[command(
    name = "excel-shrink regression",
    about = "Run real-file regression suites."
)]
pub struct RegressionCli {
    /// Write a Perfetto-compatible trace to a new JSON file.
    #[arg(long, global = true)]
    pub trace_file: Option<PathBuf>,

    /// Console log level for the regression runner.
    #[arg(long, value_enum, default_value_t = LogLevel::Info, global = true)]
    console_log_level: LogLevel,

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

    /// Run the suite once for each predefined visual profile.
    /// Baseline defaults are ignored so each run uses the exact profile configuration.
    #[arg(long)]
    all_visual_profiles: bool,

    /// Run every predefined visual profile with Excel recalculation/value and screenshot checks.
    /// This implies --all-visual-profiles, --excel-recalc-check, and --excel-screenshot-check.
    #[arg(long)]
    all_visual_profile_checks: bool,

    /// Open the shrunk workbook with Excel, force formula recalculation, save it, and compare cached cell values.
    #[arg(long)]
    excel_recalc_check: bool,

    /// Compare worksheet row heights and images in Excel.
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
    visual_profile: String,
    visual_profile_kind: String,
    visual_profile_path: String,
    config_hash: String,
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
    visual_profile: String,
    visual_profile_kind: String,
    visual_profile_path: String,
    config_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_copy_path: Option<PathBuf>,
    original_bytes: u64,
    cleaned_bytes: Option<u64>,
    reduction_ratio: Option<f64>,
    shrink_duration_ms: Option<u128>,
    original_uncompressed_bytes: Option<u64>,
    cleaned_uncompressed_bytes: Option<u64>,
    uncompressed_reduction_ratio: Option<f64>,
    analysis_mode: bool,
    failures: Vec<String>,
    sheet_size_details: Vec<SheetReductionDetail>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    original_recalculated: Option<ExcelSavedWorkbook>,
    real_recalc_differences: Vec<CellValueDifference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recalculated_baseline_vs_shrunk: Option<WorkbookComparisonReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_reference_cache_hit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_reference_cacheable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_reference_kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct AnalysisDetail {
    cause: String,
    sheet_file: Option<String>,
    removed_items: f64,
    removed_bytes_uncompressed_xml: f64,
    reduction_bytes_uncompressed_xml: f64,
}

#[derive(Debug, Clone, Serialize)]
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

struct SuiteRunResult {
    report_path: PathBuf,
    measurement_profile: RegressionMeasurementProfile,
    any_failure: bool,
}

impl SuiteRunResult {
    fn ensure_success(self) -> anyhow::Result<Self> {
        anyhow::ensure!(!self.any_failure, "One or more regression checks failed.");
        Ok(self)
    }
}

#[derive(Debug, Serialize)]
struct RegressionMeasurementReport {
    mode: String,
    created_at_unix_ms: u128,
    output_root: PathBuf,
    profiles: Vec<RegressionMeasurementProfile>,
}

#[derive(Debug, Serialize)]
struct RegressionMeasurementProfile {
    visual_profile: String,
    visual_profile_kind: String,
    visual_profile_path: String,
    config_hash: String,
    report_path: PathBuf,
    cases: Vec<RegressionCaseMeasurement>,
    aggregate: RegressionMeasurementAggregate,
}

#[derive(Debug, Serialize)]
struct RegressionCaseMeasurement {
    id: String,
    category: Option<String>,
    input_path: PathBuf,
    output_path: PathBuf,
    original_bytes: u64,
    cleaned_bytes: Option<u64>,
    reduction_ratio: Option<f64>,
    shrink_duration_ms: Option<u128>,
    original_uncompressed_bytes: Option<u64>,
    cleaned_uncompressed_bytes: Option<u64>,
    uncompressed_reduction_ratio: Option<f64>,
    sheet_size_details: Vec<SheetReductionDetail>,
    visual_diff_summary: Option<ExcelScreenshotDifferenceSummary>,
    screenshot_report_path: Option<PathBuf>,
    screenshot_output_dir: Option<PathBuf>,
    entropy_changed_diff_details: Vec<RegressionVisualDiffDetail>,
    open_timings: Vec<WorkbookOpenTiming>,
}

#[derive(Debug, Clone, Serialize)]
struct RegressionVisualDiffDetail {
    screenshot_report_path: PathBuf,
    sheet_index: usize,
    sheet_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_pixel_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approximate_cells: Option<MeasurementApproximateCells>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_crop: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_crop: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_pixels: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vertical_reflow_proof: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vertical_reflow_residual_diff_pixels: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shrunk_image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_xml_diff: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sheet_xml_visual_diff: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MeasurementApproximateCells {
    top_left: String,
    bottom_right: String,
}

#[derive(Debug, Deserialize)]
struct MeasurementScreenshotDifference {
    #[serde(default)]
    kind: Option<String>,
    sheet_index: usize,
    sheet_name: String,
    #[serde(default)]
    tile_index: Option<usize>,
    #[serde(default)]
    sheet_file: Option<String>,
    #[serde(default)]
    screenshot_error: Option<String>,
    #[serde(default)]
    diff_pixel_count: Option<u64>,
    #[serde(default)]
    approximate_cells: Option<MeasurementApproximateCells>,
    #[serde(default)]
    original_crop: Option<PathBuf>,
    #[serde(default)]
    shrunk_crop: Option<PathBuf>,
    #[serde(default)]
    diff_pixels: Option<PathBuf>,
    #[serde(default)]
    vertical_reflow_proof: Option<serde_json::Value>,
    #[serde(default)]
    original_image: Option<PathBuf>,
    #[serde(default)]
    shrunk_image: Option<PathBuf>,
    #[serde(default)]
    sheet_xml_diff: Option<PathBuf>,
    #[serde(default)]
    sheet_xml_visual_diff: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize)]
struct RegressionMeasurementAggregate {
    case_count: usize,
    successful_case_count: usize,
    total_original_bytes: u64,
    total_cleaned_bytes: u64,
    total_reduction_ratio: f64,
    total_original_uncompressed_bytes: u64,
    total_cleaned_uncompressed_bytes: u64,
    total_uncompressed_reduction_ratio: f64,
    total_original_sheet_xml_bytes: u64,
    total_cleaned_sheet_xml_bytes: u64,
    total_sheet_xml_reduction_ratio: f64,
    visual_total_differences: usize,
    visual_row_height_differences: usize,
    visual_shift_only_differences: usize,
    visual_entropy_changed_differences: usize,
    visual_screenshot_error_differences: usize,
    visual_diff_pixels: u64,
    visual_shift_movement_px: u64,
    total_shrink_duration_ms: u128,
    avg_shrink_duration_ms: Option<f64>,
    avg_original_open_ms: Option<f64>,
    avg_shrunk_open_ms: Option<f64>,
}

static DOWNLOAD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn run(cli: RegressionCli) -> anyhow::Result<()> {
    let console_log_level = cli.console_log_level;
    match cli.command {
        RegressionCommand::Smoke(args) => run_suite_or_visual_profile_matrix(
            "smoke",
            "regression/baselines/smoke.json",
            args,
            console_log_level,
        ),
        RegressionCommand::Corpus(args) => run_suite_or_visual_profile_matrix(
            "corpus",
            "regression/baselines/corpus.json",
            args,
            console_log_level,
        ),
    }
}

#[cfg(test)]
pub fn run_smoke_case_for_corpus_regression_test(case_id: &str) {
    init_corpus_regression_logging_for_test();

    let args = corpus_regression_run_args_for_test(Some(case_id.to_string()));
    run_suite(
        "smoke",
        "regression/baselines/smoke.json",
        args,
        LogLevel::Info,
    )
    .and_then(SuiteRunResult::ensure_success)
    .unwrap_or_else(|e| panic!("corpus regression smoke case '{case_id}' failed: {e:#}"));
}

#[cfg(test)]
pub fn run_smoke_suite_for_corpus_regression_test() {
    init_corpus_regression_logging_for_test();

    let args = corpus_regression_run_args_for_test(None);
    run_suite(
        "smoke",
        "regression/baselines/smoke.json",
        args,
        LogLevel::Info,
    )
    .and_then(SuiteRunResult::ensure_success)
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
        all_visual_profiles: false,
        all_visual_profile_checks: false,
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
                "EXCEL_SHRINK_CORPUS_REGRESSION_REMOVE_TEXTLESS_DEFAULT_FONT_REPRESENTATIVES",
            ),
        };

    if !keep_empty_style_sentinel_cells {
        args.push("--remove-textless-default-font-representatives".to_string());
    }

    if env_flag_enabled_for_test(
        "EXCEL_SHRINK_CORPUS_REGRESSION_REMOVE_UNREFERENCED_WHITESPACE_CELLS",
    ) {
        args.push("--remove-unreferenced-whitespace-cells".to_string());
    }

    if env_flag_enabled_for_test(
        "EXCEL_SHRINK_CORPUS_REGRESSION_KEEP_UNREFERENCED_HIDDEN_CONTENT_CELLS",
    ) {
        args.push("--keep-unreferenced-hidden-content-cells".to_string());
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
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case, LogLevel::Info)
        .unwrap_or_else(|e| panic!("failed to resolve seeschifffahrt input: {e:#}"));
    let output_root = repo_root
        .join(".tmp").join("regression")
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
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case, LogLevel::Info)
        .unwrap_or_else(|e| panic!("failed to resolve kernhaushalt input: {e:#}"));
    let output_root = repo_root
        .join(".tmp").join("regression")
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
    let input = resolve_case_input(&repo_root, corpus_root.as_deref(), case, LogLevel::Info)
        .unwrap_or_else(|e| panic!("failed to resolve {label} input: {e:#}"));
    let output_root = repo_root
        .join(".tmp").join("regression")
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
    console_log_level: LogLevel,
) -> anyhow::Result<SuiteRunResult> {
    let repo_root = repo_root();
    let requested_baseline_path = args.baseline_path.clone();
    let mut baseline_path = resolve_path(
        &repo_root,
        requested_baseline_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(default_baseline_relative_path)),
    );
    let mut suite = load_suite(&baseline_path);
    let mut suite_profile = suite_visual_profile(
        suite_defaults_for_run(&suite, args.ignore_default_cli_args),
        &args.cli_args,
    );

    if requested_baseline_path.is_none() {
        let profiled_baseline_path = profiled_regression_baseline_path(
            &repo_root,
            default_baseline_relative_path,
            &suite_profile,
        );
        if profiled_baseline_path.exists() && profiled_baseline_path != baseline_path {
            baseline_path = profiled_baseline_path;
            suite = load_suite(&baseline_path);
            suite_profile = suite_visual_profile(
                suite_defaults_for_run(&suite, args.ignore_default_cli_args),
                &args.cli_args,
            );
        }
    }

    let mut corpus_root = args
        .corpus_root
        .clone()
        .or_else(|| std::env::var_os("EXCEL_SHRINK_CORPUS_ROOT").map(PathBuf::from))
        .map(|path| resolve_path(&repo_root, path));
    let show_info = console_log_level.includes(LogLevel::Info);

    let mut cases = suite.cases.clone();
    if cases.is_empty() && mode_name == "corpus" {
        let discovery_root = corpus_root
            .clone()
            .or_else(|| existing_default_corpus_root(&repo_root))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "corpus baseline contains no cases and no corpus root exists; pass --corpus-root"
                )
            })?;
        if show_info {
            eprintln!(
                "[{mode_name}] Discovering corpus workbooks under {}",
                discovery_root.display()
            );
        }
        cases = discover_corpus_cases(&discovery_root)?;
        corpus_root = Some(discovery_root);
        if show_info {
            eprintln!("[{mode_name}] Discovered {} corpus workbooks", cases.len());
        }
    }

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
        .clone()
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| {
            repo_root
                .join(".tmp").join("regression")
                .join(format!("{mode_name}-{timestamp}"))
        });
    fs::create_dir_all(&output_root)?;

    let report_path = args
        .report_path
        .clone()
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| {
            regression_report_root(&repo_root, &suite_profile)
                .join(format!("{mode_name}-{timestamp}.json"))
        });
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
        write_visual_profile_metadata(&parent.join("profile-metadata.json"), &suite_profile, None)?;
    }
    write_visual_profile_metadata(
        &profile_artifact_root(&repo_root, "baselines", &suite_profile)
            .join("profile-metadata.json"),
        &suite_profile,
        None,
    )?;

    let mut reports = Vec::new();
    let mut any_failure = false;
    let suite_defaults = suite_defaults_for_run(&suite, args.ignore_default_cli_args);

    for case in cases {
        if show_info {
            eprintln!("[{mode_name}] Running case '{}'", case.id);
        }
        let report = run_case(
            mode_name,
            &repo_root,
            &output_root,
            corpus_root.as_deref(),
            suite_defaults,
            &args.cli_args,
            args.excel_recalc_check,
            args.excel_screenshot_check,
            console_log_level,
            &case,
        );
        match &report {
            Ok(report) if report.failures.is_empty() => {
                if show_info {
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
            }
            Ok(report) => {
                any_failure = true;
                eprintln!("[{mode_name}] FAIL {}", report.id);
                for failure in &report.failures {
                    eprintln!("  - {failure}");
                }
                if show_info && !report.top_sheet_reductions.is_empty() {
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
            if show_info {
                eprintln!(
                    "[{mode_name}] Aggregate: output={} bytes, reduction={:.2}%",
                    aggregate.total_cleaned_bytes,
                    aggregate.total_reduction_ratio * 100.0
                );
            }
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
        visual_profile: suite_profile.profile_name().to_string(),
        visual_profile_kind: suite_profile.profile_kind().to_string(),
        visual_profile_path: suite_profile.profile_path_string(),
        config_hash: suite_profile.config_hash.clone(),
        excel_recalc_check: args.excel_recalc_check,
        excel_screenshot_check: args.excel_screenshot_check,
        output_root,
        created_at_unix_ms: unix_ms(),
        cases: reports,
        aggregate,
    };
    let measurement_profile = measurement_profile_from_report(&report, &report_path);
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    if show_info {
        eprintln!("[{mode_name}] Report written to {}", report_path.display());
    }

    Ok(SuiteRunResult {
        report_path,
        measurement_profile,
        any_failure,
    })
}

fn run_suite_or_visual_profile_matrix(
    mode_name: &str,
    default_baseline_relative_path: &str,
    mut args: RegressionRunArgs,
    console_log_level: LogLevel,
) -> anyhow::Result<()> {
    if args.all_visual_profile_checks {
        args.all_visual_profiles = true;
        args.excel_recalc_check = true;
        args.excel_screenshot_check = true;
    }

    if args.all_visual_profiles {
        run_visual_profile_matrix(
            mode_name,
            default_baseline_relative_path,
            args,
            console_log_level,
        )
    } else {
        run_suite(
            mode_name,
            default_baseline_relative_path,
            args,
            console_log_level,
        )
        .and_then(SuiteRunResult::ensure_success)
        .map(|_| ())
    }
}

fn run_visual_profile_matrix(
    mode_name: &str,
    default_baseline_relative_path: &str,
    args: RegressionRunArgs,
    console_log_level: LogLevel,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.report_path.is_none(),
        "--report-path cannot be used with --all-visual-profiles because each profile writes its own report"
    );
    ensure_no_visual_profile_cli_args(&args.cli_args)?;

    let repo_root = repo_root();
    let matrix_timestamp = timestamp();
    let base_output_root = args
        .output_root
        .clone()
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| {
            repo_root.join(".tmp").join("regression").join(format!(
                "{mode_name}-all-visual-profiles-{matrix_timestamp}"
            ))
        });
    let show_info = console_log_level.includes(LogLevel::Info);
    let mut failures = Vec::new();
    let mut measurement_profiles = Vec::new();

    for predefined in predefined_visual_profiles() {
        let profile = ResolvedVisualProfile::from_config(predefined.config);
        if show_info {
            eprintln!(
                "[{mode_name}] Running visual profile '{}'",
                profile.profile_path_string()
            );
        }

        let mut profile_args = args.clone();
        profile_args.all_visual_profiles = false;
        profile_args.all_visual_profile_checks = false;
        profile_args.ignore_default_cli_args = true;
        profile_args.report_path = None;
        profile_args.output_root = Some(base_output_root.join(profile.profile_path()));
        profile_args
            .cli_args
            .extend(predefined.config.cli_args().into_iter());

        match run_suite(
            mode_name,
            default_baseline_relative_path,
            profile_args,
            console_log_level,
        ) {
            Ok(result) => {
                if result.any_failure {
                    failures.push(format!(
                        "{}: one or more regression checks failed; report: {}",
                        predefined.name,
                        result.report_path.display()
                    ));
                }
                measurement_profiles.push(result.measurement_profile);
            }
            Err(error) => {
                failures.push(format!("{}: {error:#}", predefined.name));
            }
        }
    }

    let measurement_report = RegressionMeasurementReport {
        mode: mode_name.to_string(),
        created_at_unix_ms: unix_ms(),
        output_root: base_output_root,
        profiles: measurement_profiles,
    };
    let measurement_dir = repo_root.join("regression/measurements");
    fs::create_dir_all(&measurement_dir)?;
    let measurement_path = measurement_dir.join(format!(
        "{mode_name}-all-visual-profiles-{matrix_timestamp}.json"
    ));
    let chart_path = measurement_dir.join(format!(
        "{mode_name}-all-visual-profiles-{matrix_timestamp}.html"
    ));
    fs::write(
        &measurement_path,
        serde_json::to_string_pretty(&measurement_report)?,
    )?;
    fs::write(&chart_path, measurement_chart_html(&measurement_report))?;
    if show_info {
        eprintln!(
            "[{mode_name}] Measurement report written to {}",
            measurement_path.display()
        );
        eprintln!(
            "[{mode_name}] Measurement chart written to {}",
            chart_path.display()
        );
    }

    anyhow::ensure!(
        failures.is_empty(),
        "One or more visual profile runs failed:\n{}",
        failures.join("\n")
    );
    Ok(())
}

fn ensure_no_visual_profile_cli_args(cli_args: &[String]) -> anyhow::Result<()> {
    let mut index = 0;
    while index < cli_args.len() {
        let arg = &cli_args[index];
        if visual_profile_cli_arg_name(arg).is_some() {
            anyhow::bail!(
                "--all-visual-profiles supplies visual cleanup args itself; remove '{}' from --cli-arg",
                arg
            );
        }
        index += 1;
    }
    Ok(())
}

fn visual_profile_cli_arg_name(arg: &str) -> Option<&str> {
    let stripped = arg.strip_prefix("--")?;
    let name = stripped.split_once('=').map_or(stripped, |(name, _)| name);
    match name {
        "remove-textless-default-font-representatives"
        | "remove-textless-style-representatives"
        | "remove-textless-column-style-representatives"
        | "inactive-protection-cell-policy"
        | "remove-unreferenced-whitespace-cells"
        | "keep-unreferenced-hidden-content-cells" => Some(name),
        _ => None,
    }
}

fn suite_defaults_for_run(
    suite: &RegressionSuite,
    ignore_default_cli_args: bool,
) -> Option<&SuiteDefaults> {
    if ignore_default_cli_args {
        None
    } else {
        suite.defaults.as_ref()
    }
}

fn suite_visual_profile(
    defaults: Option<&SuiteDefaults>,
    extra_cli_args: &[String],
) -> ResolvedVisualProfile {
    let mut args = vec![
        "excel-shrink".to_string(),
        "input.xlsx".to_string(),
        "out".to_string(),
    ];
    if let Some(defaults) = defaults {
        if let Some(default_args) = &defaults.cli_args {
            args.extend(default_args.iter().cloned());
        }
    }
    args.extend(extra_cli_args.iter().cloned());

    ResolvedVisualProfile::from_cli(&Cli::parse_from(args))
}

fn profiled_regression_baseline_path(
    repo_root: &Path,
    default_baseline_relative_path: &str,
    profile: &ResolvedVisualProfile,
) -> PathBuf {
    let file_name = Path::new(default_baseline_relative_path)
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("baseline.json"));

    repo_root
        .join("regression")
        .join("baselines")
        .join(profile.profile_path())
        .join(file_name)
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
    console_log_level: LogLevel,
    case: &RegressionCase,
) -> anyhow::Result<CaseReport> {
    let _phase = visual_diagnostics::phase("regression.case", &case.id);
    let source_input = resolve_case_input(repo_root, corpus_root, case, console_log_level)?;
    let case_output_dir = output_root.join(&case.id);
    fs::create_dir_all(&case_output_dir)?;
    // Keep ordinary cleanup/statistics tests unchanged. Excel-backed validation
    // starts from an explicit workbook.xml-only repaired, immutable baseline.
    let input = if excel_recalc_check || excel_screenshot_check {
        let prepared = artifact_cache::prepare_baseline(&source_input)?;
        artifact_cache::atomic_write(&case_output_dir.join("baseline-provenance.json"),
            &serde_json::to_vec_pretty(&prepared)?)?;
        prepared.snapshot.path
    } else {
        source_input.clone()
    };
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
    let visual_profile = ResolvedVisualProfile::from_cli(&cli);
    let output_path = case_output_dir.join(
        input
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("input file has no file name: {}", input.display()))?,
    );

    let mut failures = Vec::new();
    fs::remove_file(&output_path).ok();
    let shrink_started = Instant::now();
    let shrink_result = {
        let _phase = visual_diagnostics::phase("regression.shrink", input.display().to_string());
        process_files(vec![input.clone()], &cli)
    };
    let shrink_duration_ms = Some(shrink_started.elapsed().as_millis());
    if let Err(error) = shrink_result {
        failures.push(format!("excel-shrink failed: {error:#}"));
    } else if output_path.exists() {
        if excel_recalc_check || excel_screenshot_check {
            if let Err(error) = artifact_cache::assert_shrink_invariants(
                &input, &output_path, &case_output_dir.join("package-audit")) {
                failures.push(format!("package invariant failed: {error:#}"));
            }
            let history_key = format!("{}:{}", case.id, visual_profile.profile_name());
            if let Err(error) = artifact_cache::record_output(&source_input, &history_key,
                &output_path, &case_output_dir.join("output-history")) {
                failures.push(format!("output history failed: {error:#}"));
            }
        }
        if let Err(error) = excel_automation::replace_workbook_with_shared_cache_link(
            &output_path,
            &case_output_dir,
        ) {
            failures.push(format!(
                "failed to link cleaned workbook through shared cache: {error:#}"
            ));
        }
    }

    let original_bytes = fs::metadata(&input)?.len();
    let mut original_uncompressed_bytes = None;
    let mut cleaned_bytes = None;
    let mut reduction_ratio = None;
    let mut cleaned_uncompressed_bytes = None;
    let mut uncompressed_reduction_ratio = None;
    let mut sheet_size_details = Vec::new();
    let mut top_sheet_reductions = Vec::new();
    let mut analysis_details = Vec::new();
    let mut value_comparison_details = None;
    let mut excel_recalc_details = None;
    let mut excel_screenshot_details = None;

    match workbook_uncompressed_bytes(&input) {
        Ok(bytes) => original_uncompressed_bytes = Some(bytes),
        Err(error) => failures.push(format!(
            "original workbook uncompressed size analysis failed: {error:#}"
        )),
    }

    if output_path.exists() {
        let output_bytes = fs::metadata(&output_path)?.len();
        let ratio = if original_bytes == 0 {
            0.0
        } else {
            (original_bytes as f64 - output_bytes as f64) / original_bytes as f64
        };
        cleaned_bytes = Some(output_bytes);
        reduction_ratio = Some(ratio);
        match workbook_uncompressed_bytes(&output_path) {
            Ok(bytes) => {
                cleaned_uncompressed_bytes = Some(bytes);
                uncompressed_reduction_ratio =
                    original_uncompressed_bytes.map(|original| ratio_reduced(original, bytes));
            }
            Err(error) => failures.push(format!(
                "cleaned workbook uncompressed size analysis failed: {error:#}"
            )),
        }
        match sheet_size_details_by_uncompressed_xml(&input, &output_path) {
            Ok(details) => {
                top_sheet_reductions = top_sheet_reductions_from_details(&details, 5);
                sheet_size_details = details;
            }
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

        if value_comparison_details.as_ref().is_some_and(|report| report.differences.is_empty()) {
            let screenshot_dir = case_output_dir.join("excel-screenshots");
            match visual_validation::compare_sheet_visuals(&input, &output_path, &screenshot_dir) {
                Ok(report) => {
                    if report.differences_found {
                        failures.push(format!(
                            "Excel visual comparison found differences; report: {}",
                            report.html_report_path.display()
                        ));
                    }
                    excel_screenshot_details = Some(report);
                }
                Err(error) => failures.push(format!("Excel visual comparison failed: {error:#}")),
            }
        }
    }

    let mut original_copy_path = None;
    if !failures.is_empty() {
        match link_original_input_for_failed_case(&input, &case_output_dir) {
            Ok(path) => original_copy_path = Some(path),
            Err(error) => failures.push(format!(
                "failed to link original workbook into case output directory: {error:#}"
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
        visual_profile: visual_profile.profile_name().to_string(),
        visual_profile_kind: visual_profile.profile_kind().to_string(),
        visual_profile_path: visual_profile.profile_path_string(),
        config_hash: visual_profile.config_hash,
        original_copy_path,
        original_bytes,
        cleaned_bytes,
        reduction_ratio,
        shrink_duration_ms,
        original_uncompressed_bytes,
        cleaned_uncompressed_bytes,
        uncompressed_reduction_ratio,
        analysis_mode,
        failures,
        sheet_size_details,
        top_sheet_reductions,
        analysis_details,
        value_comparison_details,
        excel_recalc_details,
        excel_screenshot_details,
    })
}

fn link_original_input_for_failed_case(
    input: &Path,
    case_output_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let file_name = file_name(input)?;
    let destination = case_output_dir.join(format!("original-{}", file_name.to_string_lossy()));
    let destination_file_name = destination.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "original workbook destination has no file name: {}",
            destination.display()
        )
    })?;
    excel_automation::link_workbook_artifact_to_shared_cache(
        input,
        case_output_dir,
        case_output_dir,
        destination_file_name,
    )
}

fn compare_original_and_shrunk_values(
    original_path: &Path,
    shrunk_path: &Path,
    cli: &Cli,
) -> anyhow::Result<WorkbookComparisonReport> {
    let _phase = visual_diagnostics::phase("regression.value_validation", original_path.display().to_string());
    xlsx_value_compare::compare_workbook_preservation(
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
    let _phase = visual_diagnostics::phase(
        "regression.recalculation_validation",
        original_path.display().to_string(),
    );
    let excel_dir = case_output_dir.join("excel-recalc");
    let automation_dir = case_output_dir.join("excel-automation");
    fs::create_dir_all(&excel_dir)?;

    let original_vs_shrunk =
        compare_original_and_shrunk_values(original_path, shrunk_path, cli)?;
    let shrunk_recalculated = excel_automation::save_recalculated_copy(
        shrunk_path,
        &excel_dir
            .join("shrunk-recalculated")
            .join(file_name(shrunk_path)?),
        &automation_dir.join("shrunk"),
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
    let mut recalculated_baseline_vs_shrunk = None;
    let mut real_recalc_differences = Vec::new();
    let mut baseline_reference_cache_hit = None;
    let mut baseline_reference_cacheable = None;
    let mut baseline_reference_kind = None;

    if shrunk_vs_recalculated.differences.is_empty() {
        tracing::info!(
            "[recalc] shrunk stored values are unchanged after Excel recalculation; baseline Excel open skipped"
        );
    } else {
        let changed_sheets = sheets_with_differences(&shrunk_vs_recalculated.differences);
        tracing::info!(
            changed_sheets = changed_sheets.len(),
            changed_values = shrunk_vs_recalculated.difference_count,
            "[recalc] shrunk values changed after recalculation; recalculating paired baseline"
        );
        let reference = excel_automation::baseline_recalculation_reference(
            original_path,
            &excel_dir,
            &automation_dir,
        )?;
        let paired = xlsx_value_compare::compare_workbook_values(
            &reference.workbook.path,
            &shrunk_recalculated.path,
            missing_cell_policy(cli),
            Some(&changed_sheets),
        )?;
        real_recalc_differences = paired.differences.clone();
        if !real_recalc_differences.is_empty() {
            failures.push(format!(
                "recalculated baseline-vs-shrunk comparison found {} differences: {}{}. No address-only volatile exemption was applied.",
                real_recalc_differences.len(),
                summarize_cell_differences(&real_recalc_differences),
                if reference.cacheable {
                    ""
                } else {
                    "; this workbook has unverified/volatile/external calculation inputs; differences may be environmental, but are not a PASS"
                }
            ));
        }
        baseline_reference_cache_hit = Some(reference.cache_hit);
        baseline_reference_cacheable = Some(reference.cacheable);
        baseline_reference_kind = Some(reference.kind);
        original_recalculated = Some(reference.workbook);
        recalculated_baseline_vs_shrunk = Some(paired);
    }

    Ok((
        ExcelRecalcValidationReport {
            original_vs_shrunk,
            shrunk_recalculated,
            shrunk_vs_recalculated,
            original_recalculated,
            real_recalc_differences,
            recalculated_baseline_vs_shrunk,
            baseline_reference_cache_hit,
            baseline_reference_cacheable,
            baseline_reference_kind,
        },
        failures,
    ))
}

fn missing_cell_policy(cli: &Cli) -> MissingCellPolicy {
    MissingCellPolicy {
        allow_whitespace_values: cli.remove_unreferenced_whitespace_cells,
        allow_hidden_rows_or_columns: !cli.keep_unreferenced_hidden_content_cells,
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

fn failed_case_report(
    repo_root: &Path,
    output_root: &Path,
    case: &RegressionCase,
    failure: String,
) -> CaseReport {
    let visual_profile = ResolvedVisualProfile::from_config(VisualProfileConfig::default());
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
        visual_profile: visual_profile.profile_name().to_string(),
        visual_profile_kind: visual_profile.profile_kind().to_string(),
        visual_profile_path: visual_profile.profile_path_string(),
        config_hash: visual_profile.config_hash,
        original_copy_path: None,
        original_bytes: 0,
        cleaned_bytes: None,
        reduction_ratio: None,
        shrink_duration_ms: None,
        original_uncompressed_bytes: None,
        cleaned_uncompressed_bytes: None,
        uncompressed_reduction_ratio: None,
        analysis_mode: false,
        failures: vec![failure],
        sheet_size_details: Vec::new(),
        top_sheet_reductions: Vec::new(),
        analysis_details: Vec::new(),
        value_comparison_details: None,
        excel_recalc_details: None,
        excel_screenshot_details: None,
    }
}

fn sheet_size_details_by_uncompressed_xml(
    original_path: &Path,
    cleaned_path: &Path,
) -> anyhow::Result<Vec<SheetReductionDetail>> {
    let _phase = visual_diagnostics::phase("regression.sheet_sizes", "ZIP entry sizes");
    let original_sheets = worksheet_xml_sizes(original_path)?;
    let cleaned_sheets = worksheet_xml_sizes(cleaned_path)?;

    let mut sheet_files = original_sheets
        .keys()
        .chain(cleaned_sheets.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    sheet_files.sort();

    let mut details = Vec::new();
    for sheet_file in sheet_files {
        let original_size = original_sheets.get(&sheet_file).copied().unwrap_or(0);
        let cleaned_size = cleaned_sheets.get(&sheet_file).copied().unwrap_or(0);
        let reduction_bytes = original_size as i64 - cleaned_size as i64;

        details.push(SheetReductionDetail {
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

    Ok(details)
}

fn top_sheet_reductions_from_details(
    details: &[SheetReductionDetail],
    limit: usize,
) -> Vec<SheetReductionDetail> {
    let mut reductions = details
        .iter()
        .filter(|detail| detail.reduction_bytes_uncompressed_xml > 0)
        .cloned()
        .collect::<Vec<_>>();
    reductions.sort_by(|left, right| {
        right
            .reduction_bytes_uncompressed_xml
            .cmp(&left.reduction_bytes_uncompressed_xml)
            .then_with(|| left.sheet_file.cmp(&right.sheet_file))
    });
    reductions.truncate(limit);
    reductions
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

fn workbook_uncompressed_bytes(path: &Path) -> anyhow::Result<u64> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut total = 0_u64;

    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        total = total.saturating_add(entry.size());
    }

    Ok(total)
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

fn measurement_profile_from_report(
    report: &RegressionReport,
    report_path: &Path,
) -> RegressionMeasurementProfile {
    let _phase = visual_diagnostics::phase("regression.measurement_report", "measurement aggregation");
    let cases = report
        .cases
        .iter()
        .map(|case| {
            let screenshot_report_path = case
                .excel_screenshot_details
                .as_ref()
                .map(|details| details.report_path.clone());
            RegressionCaseMeasurement {
                id: case.id.clone(),
                category: case.category.clone(),
                input_path: case.input_path.clone(),
                output_path: case.output_path.clone(),
                original_bytes: case.original_bytes,
                cleaned_bytes: case.cleaned_bytes,
                reduction_ratio: case.reduction_ratio,
                shrink_duration_ms: case.shrink_duration_ms,
                original_uncompressed_bytes: case.original_uncompressed_bytes,
                cleaned_uncompressed_bytes: case.cleaned_uncompressed_bytes,
                uncompressed_reduction_ratio: case.uncompressed_reduction_ratio,
                sheet_size_details: case.sheet_size_details.clone(),
                visual_diff_summary: case
                    .excel_screenshot_details
                    .as_ref()
                    .map(|details| details.difference_summary.clone()),
                screenshot_report_path: screenshot_report_path.clone(),
                screenshot_output_dir: case
                    .excel_screenshot_details
                    .as_ref()
                    .map(|details| details.output_dir.clone()),
                entropy_changed_diff_details: screenshot_report_path
                    .as_deref()
                    .map(load_entropy_changed_diff_details)
                    .unwrap_or_default(),
                open_timings: case
                    .excel_screenshot_details
                    .as_ref()
                    .map(|details| details.open_timings.clone())
                    .unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    let aggregate = measurement_aggregate(&cases);

    RegressionMeasurementProfile {
        visual_profile: report.visual_profile.clone(),
        visual_profile_kind: report.visual_profile_kind.clone(),
        visual_profile_path: report.visual_profile_path.clone(),
        config_hash: report.config_hash.clone(),
        report_path: report_path.to_path_buf(),
        cases,
        aggregate,
    }
}

fn measurement_aggregate(cases: &[RegressionCaseMeasurement]) -> RegressionMeasurementAggregate {
    let successful = cases
        .iter()
        .filter(|case| case.cleaned_bytes.is_some())
        .collect::<Vec<_>>();
    let total_original_bytes = successful
        .iter()
        .map(|case| case.original_bytes)
        .sum::<u64>();
    let total_cleaned_bytes = successful
        .iter()
        .filter_map(|case| case.cleaned_bytes)
        .sum::<u64>();
    let total_original_uncompressed_bytes = successful
        .iter()
        .filter_map(|case| case.original_uncompressed_bytes)
        .sum::<u64>();
    let total_cleaned_uncompressed_bytes = successful
        .iter()
        .filter_map(|case| case.cleaned_uncompressed_bytes)
        .sum::<u64>();
    let total_original_sheet_xml_bytes = successful
        .iter()
        .flat_map(|case| &case.sheet_size_details)
        .map(|sheet| sheet.original_sheet_xml_bytes)
        .sum::<u64>();
    let total_cleaned_sheet_xml_bytes = successful
        .iter()
        .flat_map(|case| &case.sheet_size_details)
        .map(|sheet| sheet.cleaned_sheet_xml_bytes)
        .sum::<u64>();

    let mut aggregate = RegressionMeasurementAggregate {
        case_count: cases.len(),
        successful_case_count: successful.len(),
        total_original_bytes,
        total_cleaned_bytes,
        total_reduction_ratio: ratio_reduced(total_original_bytes, total_cleaned_bytes),
        total_original_uncompressed_bytes,
        total_cleaned_uncompressed_bytes,
        total_uncompressed_reduction_ratio: ratio_reduced(
            total_original_uncompressed_bytes,
            total_cleaned_uncompressed_bytes,
        ),
        total_original_sheet_xml_bytes,
        total_cleaned_sheet_xml_bytes,
        total_sheet_xml_reduction_ratio: ratio_reduced(
            total_original_sheet_xml_bytes,
            total_cleaned_sheet_xml_bytes,
        ),
        total_shrink_duration_ms: successful
            .iter()
            .filter_map(|case| case.shrink_duration_ms)
            .sum(),
        avg_shrink_duration_ms: average_shrink_duration_ms(&successful),
        avg_original_open_ms: average_open_duration_ms(cases, "original"),
        avg_shrunk_open_ms: average_open_duration_ms(cases, "shrunk"),
        ..RegressionMeasurementAggregate::default()
    };

    for summary in cases
        .iter()
        .filter_map(|case| case.visual_diff_summary.as_ref())
    {
        aggregate.visual_total_differences += summary.total_differences;
        aggregate.visual_row_height_differences += summary.row_height_differences;
        aggregate.visual_shift_only_differences += summary.shift_only_differences;
        aggregate.visual_entropy_changed_differences += summary.entropy_changed_differences;
        aggregate.visual_screenshot_error_differences += summary.screenshot_error_differences;
        aggregate.visual_diff_pixels = aggregate
            .visual_diff_pixels
            .saturating_add(summary.diff_pixels);
        aggregate.visual_shift_movement_px = aggregate
            .visual_shift_movement_px
            .saturating_add(summary.shift_movement_px);
    }

    aggregate
}

fn ratio_reduced(original: u64, cleaned: u64) -> f64 {
    if original == 0 {
        0.0
    } else {
        (original as f64 - cleaned as f64) / original as f64
    }
}

fn load_entropy_changed_diff_details(report_path: &Path) -> Vec<RegressionVisualDiffDetail> {
    let Ok(bytes) = fs::read(report_path) else {
        return Vec::new();
    };
    let Ok(differences) = serde_json::from_slice::<Vec<MeasurementScreenshotDifference>>(&bytes)
    else {
        return Vec::new();
    };

    let mut details = differences
        .into_iter()
        .filter(|difference| {
            difference.kind.as_deref() != Some("row_height")
                && difference.screenshot_error.is_none()
                && !vertical_reflow_proof_is_shift_only(difference.vertical_reflow_proof.as_ref())
        })
        .map(|difference| {
            let vertical_reflow_residual_diff_pixels = difference
                .vertical_reflow_proof
                .as_ref()
                .and_then(vertical_reflow_residual_diff_pixels_path);
            RegressionVisualDiffDetail {
                screenshot_report_path: report_path.to_path_buf(),
                sheet_index: difference.sheet_index,
                sheet_name: difference.sheet_name,
                tile_index: difference.tile_index,
                sheet_file: difference.sheet_file,
                diff_pixel_count: difference.diff_pixel_count,
                approximate_cells: difference.approximate_cells,
                original_crop: difference.original_crop,
                shrunk_crop: difference.shrunk_crop,
                diff_pixels: difference.diff_pixels,
                vertical_reflow_proof: difference.vertical_reflow_proof,
                vertical_reflow_residual_diff_pixels,
                original_image: difference.original_image,
                shrunk_image: difference.shrunk_image,
                sheet_xml_diff: difference.sheet_xml_diff,
                sheet_xml_visual_diff: difference.sheet_xml_visual_diff,
            }
        })
        .collect::<Vec<_>>();
    details.sort_by(|left, right| {
        right
            .diff_pixel_count
            .unwrap_or(0)
            .cmp(&left.diff_pixel_count.unwrap_or(0))
    });
    details
}

fn vertical_reflow_proof_is_shift_only(proof: Option<&serde_json::Value>) -> bool {
    let Some(proof) = proof else {
        return false;
    };
    proof
        .get("reconstructs_original")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && proof
            .get("residual_diff_pixels")
            .and_then(serde_json::Value::as_u64)
            == Some(0)
        && proof
            .get("uses_only_spacer_rows")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
}

fn vertical_reflow_residual_diff_pixels_path(proof: &serde_json::Value) -> Option<PathBuf> {
    proof
        .get("residual_diff_pixels_image")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
}

fn average_open_duration_ms(cases: &[RegressionCaseMeasurement], label: &str) -> Option<f64> {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for timing in cases.iter().flat_map(|case| &case.open_timings) {
        if timing.label.eq_ignore_ascii_case(label) {
            total = total.saturating_add(timing.duration_ms);
            count += 1;
        }
    }
    (count > 0).then_some(total as f64 / count as f64)
}

fn average_shrink_duration_ms(cases: &[&RegressionCaseMeasurement]) -> Option<f64> {
    let mut total = 0u128;
    let mut count = 0usize;
    for duration_ms in cases.iter().filter_map(|case| case.shrink_duration_ms) {
        total = total.saturating_add(duration_ms);
        count += 1;
    }
    (count > 0).then(|| total as f64 / count as f64)
}

fn measurement_chart_html(report: &RegressionMeasurementReport) -> String {
    let mut html = String::new();
    let _ = writeln!(
        html,
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>excel-shrink regression measurements</title>"
    );
    let _ = writeln!(
        html,
        "<style>body{{font-family:Arial,sans-serif;margin:24px;color:#1f2933;background:#f8fafc}}h1{{font-size:22px;margin:0 0 6px}}h2{{font-size:16px;margin:28px 0 10px}}.meta{{color:#52606d;margin-bottom:18px}}svg{{background:#fff;border:1px solid #d9e2ec}}table{{border-collapse:collapse;background:#fff;margin-top:12px;width:100%;font-size:13px}}th,td{{border:1px solid #d9e2ec;padding:7px 9px;text-align:right}}th:first-child,td:first-child{{text-align:left}}th{{background:#edf2f7}}.legend{{font-size:13px;color:#323f4b;margin:8px 0 18px}}.swatch{{display:inline-block;width:11px;height:11px;margin-right:5px;vertical-align:-1px}}</style></head><body>"
    );
    let _ = writeln!(html, "<h1>excel-shrink regression measurements</h1>");
    let _ = writeln!(
        html,
        "<div class=\"meta\">mode={} profiles={} output_root={}</div>",
        html_escape(&report.mode),
        report.profiles.len(),
        html_escape(&report.output_root.display().to_string())
    );
    html.push_str("<h2>Size reduction and visual diffs</h2>");
    html.push_str("<div class=\"legend\"><span class=\"swatch\" style=\"background:#5d6675\"></span>case workbook reduction points <span class=\"swatch\" style=\"background:#1f77b4\"></span>aggregate XLSX reduction <span class=\"swatch\" style=\"background:#6b46c1\"></span>aggregate uncompressed workbook reduction <span class=\"swatch\" style=\"background:#2f855a\"></span>aggregate worksheet XML reduction <span class=\"swatch\" style=\"background:#b7791f\"></span>row height diffs <span class=\"swatch\" style=\"background:#c53030\"></span>image diffs</div>");
    html.push_str(&measurement_overview_svg(report));
    html.push_str("<h2>Excel open time</h2>");
    html.push_str("<div class=\"legend\"><span class=\"swatch\" style=\"background:#3b82f6\"></span>original/repaired original <span class=\"swatch\" style=\"background:#ef4444\"></span>shrunk</div>");
    html.push_str(&measurement_open_time_svg(report));
    html.push_str("<h2>Profile summary</h2>");
    html.push_str(&measurement_summary_table(report));
    html.push_str("<h2>Image differences</h2>");
    html.push_str(&measurement_entropy_diff_table(report));
    html.push_str("</body></html>");
    html
}

fn measurement_overview_svg(report: &RegressionMeasurementReport) -> String {
    const WIDTH: f64 = 1180.0;
    const HEIGHT: f64 = 500.0;
    const LEFT: f64 = 76.0;
    const RIGHT: f64 = 84.0;
    const TOP: f64 = 28.0;
    const BOTTOM: f64 = 108.0;

    let plot_width = WIDTH - LEFT - RIGHT;
    let plot_height = HEIGHT - TOP - BOTTOM;
    let bottom_y = HEIGHT - BOTTOM;
    let max_diff = report
        .profiles
        .iter()
        .map(|profile| {
            profile.aggregate.visual_row_height_differences
                + profile.aggregate.visual_entropy_changed_differences
        })
        .max()
        .unwrap_or(0)
        .max(1) as f64;

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg viewBox=\"0 0 {WIDTH} {HEIGHT}\" width=\"100%\" height=\"500\" role=\"img\">"
    );
    svg.push_str("<line x1=\"76\" y1=\"28\" x2=\"76\" y2=\"392\" stroke=\"#94a3b8\"/><line x1=\"76\" y1=\"392\" x2=\"1096\" y2=\"392\" stroke=\"#94a3b8\"/><line x1=\"1096\" y1=\"28\" x2=\"1096\" y2=\"392\" stroke=\"#94a3b8\"/>");
    for tick in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = bottom_y - tick * plot_height;
        let _ = writeln!(
            svg,
            "<line x1=\"70\" y1=\"{y:.1}\" x2=\"1096\" y2=\"{y:.1}\" stroke=\"#e2e8f0\"/><text x=\"62\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#52606d\">{:.0}%</text>",
            y + 4.0,
            tick * 100.0
        );
    }
    for tick in 0..=4 {
        let value = (max_diff * tick as f64 / 4.0).round();
        let y = bottom_y - (value / max_diff) * plot_height;
        let _ = writeln!(
            svg,
            "<text x=\"1104\" y=\"{:.1}\" font-size=\"11\" fill=\"#52606d\">{value:.0}</text>",
            y + 4.0
        );
    }

    let workbook_points = report
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            (
                profile_x(index, report.profiles.len(), LEFT, plot_width),
                ratio_y(
                    profile.aggregate.total_reduction_ratio,
                    bottom_y,
                    plot_height,
                ),
            )
        })
        .collect::<Vec<_>>();
    let sheet_points = report
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            (
                profile_x(index, report.profiles.len(), LEFT, plot_width),
                ratio_y(
                    profile.aggregate.total_sheet_xml_reduction_ratio,
                    bottom_y,
                    plot_height,
                ),
            )
        })
        .collect::<Vec<_>>();
    let uncompressed_workbook_points = report
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            (
                profile_x(index, report.profiles.len(), LEFT, plot_width),
                ratio_y(
                    profile.aggregate.total_uncompressed_reduction_ratio,
                    bottom_y,
                    plot_height,
                ),
            )
        })
        .collect::<Vec<_>>();
    let shift_points = report
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let x = profile_x(index, report.profiles.len(), LEFT, plot_width);
            let y = bottom_y
                - (profile.aggregate.visual_row_height_differences as f64 / max_diff) * plot_height;
            (x, y)
        })
        .collect::<Vec<_>>();
    let entropy_points = report
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let x = profile_x(index, report.profiles.len(), LEFT, plot_width);
            let y = bottom_y
                - (profile.aggregate.visual_entropy_changed_differences as f64 / max_diff)
                    * plot_height;
            (x, y)
        })
        .collect::<Vec<_>>();

    svg.push_str(&polyline_svg(&workbook_points, "#1f77b4", 3.0, None));
    svg.push_str(&polyline_svg(
        &uncompressed_workbook_points,
        "#6b46c1",
        2.5,
        None,
    ));
    svg.push_str(&polyline_svg(&sheet_points, "#2f855a", 3.0, None));
    svg.push_str(&polyline_svg(&shift_points, "#b7791f", 2.0, Some("6 4")));
    svg.push_str(&polyline_svg(&entropy_points, "#c53030", 2.0, Some("6 4")));

    for (index, profile) in report.profiles.iter().enumerate() {
        let x = profile_x(index, report.profiles.len(), LEFT, plot_width);
        let _ = writeln!(
            svg,
            "<line x1=\"{x:.1}\" y1=\"392\" x2=\"{x:.1}\" y2=\"398\" stroke=\"#94a3b8\"/><text x=\"{x:.1}\" y=\"414\" transform=\"rotate(35 {x:.1} 414)\" font-size=\"11\" fill=\"#323f4b\">{}</text>",
            svg_escape(&profile.visual_profile)
        );

        for (case_index, case) in profile.cases.iter().enumerate() {
            let Some(ratio) = case.reduction_ratio else {
                continue;
            };
            let jitter = ((case_index % 7) as f64 - 3.0) * 3.0;
            let y = ratio_y(ratio, bottom_y, plot_height);
            let _ = writeln!(
                svg,
                "<circle cx=\"{:.1}\" cy=\"{y:.1}\" r=\"4\" fill=\"#5d6675\" opacity=\"0.72\"><title>{}: xlsx {} -> {} ({}); uncompressed {} -> {}</title></circle>",
                x + jitter,
                svg_escape(&case.id),
                format_bytes(case.original_bytes),
                case.cleaned_bytes.map(format_bytes).unwrap_or_else(|| "n/a".to_string()),
                format_pct(case.reduction_ratio),
                case.original_uncompressed_bytes
                    .map(format_bytes)
                    .unwrap_or_else(|| "n/a".to_string()),
                case.cleaned_uncompressed_bytes
                    .map(format_bytes)
                    .unwrap_or_else(|| "n/a".to_string())
            );
        }
    }

    svg.push_str("<text x=\"76\" y=\"18\" font-size=\"12\" fill=\"#52606d\">size reduction</text><text x=\"1096\" y=\"18\" text-anchor=\"end\" font-size=\"12\" fill=\"#52606d\">visual diff count</text>");
    svg.push_str("</svg>");
    svg
}

fn measurement_open_time_svg(report: &RegressionMeasurementReport) -> String {
    const WIDTH: f64 = 1180.0;
    const HEIGHT: f64 = 320.0;
    const LEFT: f64 = 76.0;
    const TOP: f64 = 24.0;
    const BOTTOM: f64 = 82.0;
    const RIGHT: f64 = 42.0;

    let plot_width = WIDTH - LEFT - RIGHT;
    let plot_height = HEIGHT - TOP - BOTTOM;
    let bottom_y = HEIGHT - BOTTOM;
    let max_ms = report
        .profiles
        .iter()
        .flat_map(|profile| {
            [
                profile.aggregate.avg_original_open_ms,
                profile.aggregate.avg_shrunk_open_ms,
            ]
        })
        .flatten()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg viewBox=\"0 0 {WIDTH} {HEIGHT}\" width=\"100%\" height=\"320\" role=\"img\">"
    );
    svg.push_str("<line x1=\"76\" y1=\"24\" x2=\"76\" y2=\"238\" stroke=\"#94a3b8\"/><line x1=\"76\" y1=\"238\" x2=\"1138\" y2=\"238\" stroke=\"#94a3b8\"/>");
    for tick in 0..=4 {
        let value = max_ms * tick as f64 / 4.0;
        let y = bottom_y - (value / max_ms) * plot_height;
        let _ = writeln!(
            svg,
            "<line x1=\"70\" y1=\"{y:.1}\" x2=\"1138\" y2=\"{y:.1}\" stroke=\"#e2e8f0\"/><text x=\"62\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#52606d\">{}</text>",
            y + 4.0,
            format_ms(Some(value))
        );
    }

    let bar_width = (plot_width / report.profiles.len().max(1) as f64).min(44.0) / 3.0;
    for (index, profile) in report.profiles.iter().enumerate() {
        let x = profile_x(index, report.profiles.len(), LEFT, plot_width);
        draw_open_bar(
            &mut svg,
            x - bar_width - 2.0,
            bar_width,
            profile.aggregate.avg_original_open_ms,
            max_ms,
            bottom_y,
            plot_height,
            "#3b82f6",
            "original",
            profile,
        );
        draw_open_bar(
            &mut svg,
            x + 2.0,
            bar_width,
            profile.aggregate.avg_shrunk_open_ms,
            max_ms,
            bottom_y,
            plot_height,
            "#ef4444",
            "shrunk",
            profile,
        );
        let _ = writeln!(
            svg,
            "<text x=\"{x:.1}\" y=\"260\" transform=\"rotate(35 {x:.1} 260)\" font-size=\"11\" fill=\"#323f4b\">{}</text>",
            svg_escape(&profile.visual_profile)
        );
    }

    svg.push_str(
        "<text x=\"76\" y=\"16\" font-size=\"12\" fill=\"#52606d\">average open time</text></svg>",
    );
    svg
}

#[allow(clippy::too_many_arguments)]
fn draw_open_bar(
    svg: &mut String,
    x: f64,
    width: f64,
    value: Option<f64>,
    max_ms: f64,
    bottom_y: f64,
    plot_height: f64,
    color: &str,
    label: &str,
    profile: &RegressionMeasurementProfile,
) {
    let Some(value) = value else {
        return;
    };
    let height = (value / max_ms) * plot_height;
    let y = bottom_y - height;
    let _ = writeln!(
        svg,
        "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"{height:.1}\" fill=\"{color}\" opacity=\"0.85\"><title>{} {} {}</title></rect>",
        svg_escape(&profile.visual_profile),
        label,
        format_ms(Some(value))
    );
}

fn measurement_summary_table(report: &RegressionMeasurementReport) -> String {
    let mut table = String::new();
    table.push_str("<table><thead><tr><th>profile</th><th>cases</th><th>XLSX reduction</th><th>uncompressed workbook reduction</th><th>sheet XML reduction</th><th>shrink total</th><th>avg shrink</th><th>row height diffs</th><th>image diffs</th><th>avg original open</th><th>avg shrunk open</th></tr></thead><tbody>");
    for profile in &report.profiles {
        let _ = writeln!(
            table,
            "<tr><td>{}</td><td>{}/{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&profile.visual_profile),
            profile.aggregate.successful_case_count,
            profile.aggregate.case_count,
            format_pct(Some(profile.aggregate.total_reduction_ratio)),
            format_pct(Some(profile.aggregate.total_uncompressed_reduction_ratio)),
            format_pct(Some(profile.aggregate.total_sheet_xml_reduction_ratio)),
            format_ms(Some(profile.aggregate.total_shrink_duration_ms as f64)),
            format_ms(profile.aggregate.avg_shrink_duration_ms),
            profile.aggregate.visual_row_height_differences,
            profile.aggregate.visual_entropy_changed_differences,
            format_ms(profile.aggregate.avg_original_open_ms),
            format_ms(profile.aggregate.avg_shrunk_open_ms)
        );
    }
    table.push_str("</tbody></table>");
    table
}

fn measurement_entropy_diff_table(report: &RegressionMeasurementReport) -> String {
    let mut rows = Vec::new();
    for profile in &report.profiles {
        for case in &profile.cases {
            for detail in &case.entropy_changed_diff_details {
                rows.push((profile, case, detail));
            }
        }
    }
    rows.sort_by(|(_, _, left), (_, _, right)| {
        right
            .diff_pixel_count
            .unwrap_or(0)
            .cmp(&left.diff_pixel_count.unwrap_or(0))
    });

    if rows.is_empty() {
        return "<div class=\"meta\">No image differences were recorded.</div>"
            .to_string();
    }

    let mut table = String::new();
    table.push_str("<table><thead><tr><th>profile</th><th>case</th><th>sheet</th><th>pixels</th><th>report</th></tr></thead><tbody>");
    for (profile, case, detail) in rows {
        let sheet = match detail.tile_index {
            Some(tile_index) => format!(
                "{} {} tile {}",
                detail.sheet_index, detail.sheet_name, tile_index
            ),
            None => format!("{} {}", detail.sheet_index, detail.sheet_name),
        };
        let html_report = detail.screenshot_report_path.with_file_name("report.html");
        let report_link = if html_report.is_file() {
            file_link(&html_report, "html")
        } else {
            file_link(&detail.screenshot_report_path, "json")
        };
        let _ = writeln!(
            table,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{report_link}</td></tr>",
            html_escape(&profile.visual_profile),
            html_escape(&case.id),
            html_escape(&sheet),
            detail
                .diff_pixel_count
                .map(|pixels| pixels.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
        );
    }
    table.push_str("</tbody></table>");
    table
}

fn profile_x(index: usize, profile_count: usize, left: f64, plot_width: f64) -> f64 {
    if profile_count <= 1 {
        return left + plot_width / 2.0;
    }
    left + (index as f64 / (profile_count - 1) as f64) * plot_width
}

fn ratio_y(ratio: f64, bottom_y: f64, plot_height: f64) -> f64 {
    bottom_y - ratio.clamp(0.0, 1.0) * plot_height
}

fn polyline_svg(points: &[(f64, f64)], color: &str, width: f64, dash: Option<&str>) -> String {
    if points.is_empty() {
        return String::new();
    }
    let point_text = points
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let dash_attr = dash
        .map(|dash| format!(" stroke-dasharray=\"{}\"", svg_escape(dash)))
        .unwrap_or_default();
    format!(
        "<polyline points=\"{point_text}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash_attr}/>"
    )
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.1}%", value * 100.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_ms(value: Option<f64>) -> String {
    value
        .map(|value| {
            if value >= 1000.0 {
                format!("{:.1}s", value / 1000.0)
            } else {
                format!("{value:.0}ms")
            }
        })
        .unwrap_or_else(|| "n/a".to_string())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn svg_escape(value: &str) -> String {
    html_escape(value)
}

fn file_link(path: &Path, label: &str) -> String {
    format!(
        "<a href=\"{}\">{}</a>",
        html_escape(&file_href(path)),
        html_escape(label)
    )
}

fn file_href(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let url = if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    };
    percent_encode_file_url(&url)
}

fn percent_encode_file_url(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b':' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
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

fn existing_default_corpus_root(repo_root: &Path) -> Option<PathBuf> {
    [
        cached_corpus_root(repo_root),
        sibling_corpus_root(repo_root),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn discover_corpus_cases(corpus_root: &Path) -> anyhow::Result<Vec<RegressionCase>> {
    anyhow::ensure!(
        corpus_root.exists(),
        "corpus root does not exist: {}",
        corpus_root.display()
    );
    anyhow::ensure!(
        corpus_root.is_dir(),
        "corpus root is not a directory: {}",
        corpus_root.display()
    );

    let mut files = Vec::new();
    collect_xlsx_files(corpus_root, &mut files)?;
    files.sort();

    let mut ids = HashMap::<String, usize>::new();
    let cases = files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(corpus_root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let base_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(sanitize_case_id)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| "workbook".to_string());
            let seen = ids.entry(base_id.clone()).or_insert(0);
            *seen += 1;
            let id = if *seen == 1 {
                base_id
            } else {
                format!("{base_id}-{}", *seen)
            };

            RegressionCase {
                id,
                category: Some("Corpus".to_string()),
                reason: Some("Auto-discovered full-corpus workbook.".to_string()),
                dominant_cleanup: None,
                notes: None,
                source_repository_url: None,
                source_file_url: None,
                input_path: None,
                relative_input: Some(relative),
                download_url: None,
                cached_file_name: None,
                cli_args: None,
                analysis_mode: None,
                max_output_bytes: None,
                min_reduction_ratio: None,
                analysis_checks: None,
            }
        })
        .collect::<Vec<_>>();

    anyhow::ensure!(
        !cases.is_empty(),
        "no .xlsx files found under corpus root: {}",
        corpus_root.display()
    );

    Ok(cases)
}

fn collect_xlsx_files(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_xlsx_files(&path, files)?;
            continue;
        }
        if !file_type.is_file() || !is_xlsx_path(&path) {
            continue;
        }
        files.push(path);
    }
    Ok(())
}

fn is_xlsx_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    !file_name.starts_with("~$")
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

fn sanitize_case_id(value: &str) -> String {
    let mut id = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if !last_dash && !id.is_empty() {
                id.push(ch);
                last_dash = true;
            }
        } else {
            id.push(ch);
            last_dash = false;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    id
}

fn resolve_case_input(
    repo_root: &Path,
    corpus_root: Option<&Path>,
    case: &RegressionCase,
    console_log_level: LogLevel,
) -> anyhow::Result<PathBuf> {
    let _phase = visual_diagnostics::phase("regression.resolve_input", &case.id);
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

        if console_log_level.includes(LogLevel::Warn) {
            eprintln!(
                "[smoke] Input file for case '{}' not found under corpus root: {}",
                case.id,
                path.display()
            );
            eprintln!("[smoke] Falling back to local cache, sibling corpus, or download URL.");
        }
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

    ensure_downloaded_case_input(repo_root, case, console_log_level)
}

fn ensure_downloaded_case_input(
    repo_root: &Path,
    case: &RegressionCase,
    console_log_level: LogLevel,
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

    if console_log_level.includes(LogLevel::Info) {
        eprintln!("[download] {download_url}");
    }
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

fn regression_report_root(repo_root: &Path, profile: &ResolvedVisualProfile) -> PathBuf {
    repo_root
        .join("regression/reports")
        .join(profile.profile_path())
}

#[cfg(test)]
mod visual_profile_matrix_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn regression_cli_parses_all_visual_profile_checks() {
        let cli =
            RegressionCli::parse_from(["excel-shrink", "smoke", "--all-visual-profile-checks"]);

        match cli.command {
            RegressionCommand::Smoke(args) => {
                assert!(args.all_visual_profile_checks);
                assert!(!args.all_visual_profiles);
            }
            RegressionCommand::Corpus(_) => panic!("expected smoke command"),
        }
    }

    #[test]
    fn visual_profile_matrix_allows_non_visual_extra_cli_args() {
        ensure_no_visual_profile_cli_args(&["--store-output".to_string()])
            .expect("non-visual extra args should be accepted");
    }

    #[test]
    fn visual_profile_matrix_rejects_visual_extra_cli_args() {
        let error = ensure_no_visual_profile_cli_args(&[
            "--remove-unreferenced-whitespace-cells".to_string()
        ])
        .expect_err("visual args should be rejected");

        assert!(error
            .to_string()
            .contains("--remove-unreferenced-whitespace-cells"));
    }

    #[test]
    fn detects_visual_profile_cli_args_with_inline_values() {
        assert_eq!(
            visual_profile_cli_arg_name("--inactive-protection-cell-policy=keep-all"),
            Some("inactive-protection-cell-policy")
        );
        assert_eq!(visual_profile_cli_arg_name("--store-output"), None);
    }

    #[test]
    fn default_regression_report_root_is_not_temporary() {
        let repo_root = PathBuf::from("repo");
        let profile = ResolvedVisualProfile::from_config(
            crate::visual_profile::VisualProfileConfig::default(),
        );

        assert_eq!(
            regression_report_root(&repo_root, &profile),
            PathBuf::from("repo").join("regression/reports/default")
        );
    }

    #[test]
    fn entropy_diff_loader_ignores_shift_only_and_screenshot_errors() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-entropy-diff-index-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir)?;
        let report_path = test_dir.join("screenshot-differences.json");
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&serde_json::json!([
                {
                    "sheet_index": 1,
                    "sheet_name": "Changed",
                    "diff_pixel_count": 42,
                    "diff_pixels": test_dir.join("1-diff-pixels.png"),
                    "approximate_cells": {
                        "top_left": "A1",
                        "bottom_right": "B2"
                    }
                },
                {
                    "sheet_index": 2,
                    "sheet_name": "Shift",
                    "diff_pixel_count": 200,
                    "vertical_reflow_proof": {
                        "classification": "vertical_suffix_reflow",
                        "reconstructs_original": true,
                        "residual_diff_pixels": 0
                    }
                },
                {
                    "sheet_index": 4,
                    "sheet_name": "Repair",
                    "diff_pixel_count": 12,
                    "vertical_reflow_proof": {
                        "classification": "vertical_suffix_reflow_repair",
                        "reconstructs_original": false,
                        "residual_diff_pixels": 3,
                        "residual_diff_pixels_image": test_dir.join("4-residual.png")
                    }
                },
                {
                    "sheet_index": 3,
                    "sheet_name": "Error",
                    "screenshot_error": "capture failed"
                }
            ]))?,
        )?;

        let details = load_entropy_changed_diff_details(&report_path);

        assert_eq!(details.len(), 2);
        assert_eq!(details[0].sheet_index, 1);
        assert_eq!(details[0].diff_pixel_count, Some(42));
        assert_eq!(
            details[0]
                .approximate_cells
                .as_ref()
                .map(|cells| (cells.top_left.as_str(), cells.bottom_right.as_str())),
            Some(("A1", "B2"))
        );
        assert_eq!(details[1].sheet_index, 4);
        assert_eq!(
            details[1].vertical_reflow_residual_diff_pixels.as_deref(),
            Some(test_dir.join("4-residual.png").as_path())
        );

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }

    #[test]
    fn discovers_corpus_cases_from_xlsx_tree() -> anyhow::Result<()> {
        let test_dir = std::env::temp_dir().join(format!(
            "excel-shrink-corpus-discovery-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(test_dir.join("nested"))?;
        fs::write(test_dir.join("Alpha.xlsx"), b"")?;
        fs::write(test_dir.join("~$Alpha.xlsx"), b"")?;
        fs::write(test_dir.join("nested").join("Beta File.xlsx"), b"")?;
        fs::write(test_dir.join("nested").join("ignore.txt"), b"")?;

        let cases = discover_corpus_cases(&test_dir)?;

        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].id, "alpha");
        assert_eq!(cases[0].relative_input.as_deref(), Some("Alpha.xlsx"));
        assert_eq!(cases[1].id, "beta-file");
        assert_eq!(
            cases[1].relative_input.as_deref(),
            Some("nested/Beta File.xlsx")
        );

        let _ = fs::remove_dir_all(&test_dir);
        Ok(())
    }
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
