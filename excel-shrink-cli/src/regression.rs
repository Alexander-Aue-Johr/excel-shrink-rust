use crate::pipeline::process_files;
use crate::Cli;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

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

    /// Include slow smoke cases.
    #[arg(long)]
    include_slow: bool,

    /// Allow a selected baseline to contain no cases.
    #[arg(long)]
    allow_empty: bool,

    /// Ignore the baseline-level defaults.cli_args for this run.
    #[arg(long)]
    ignore_default_cli_args: bool,

    /// Extra CLI argument passed to excel-shrink for each case. Repeat for multiple args.
    #[arg(long = "cli-arg", allow_hyphen_values = true)]
    cli_args: Vec<String>,
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
    slow: Option<bool>,
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
    include_slow: bool,
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
    slow: bool,
    input_path: PathBuf,
    output_path: PathBuf,
    original_bytes: u64,
    cleaned_bytes: Option<u64>,
    reduction_ratio: Option<f64>,
    analysis_mode: bool,
    failures: Vec<String>,
    analysis_details: Vec<AnalysisDetail>,
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

    let args = RegressionRunArgs {
        baseline_path: None,
        corpus_root: None,
        output_root: None,
        report_path: None,
        case_id: Some(case_id.to_string()),
        include_slow: true,
        allow_empty: false,
        ignore_default_cli_args: env_flag_enabled_for_test(
            "EXCEL_SHRINK_CORPUS_REGRESSION_IGNORE_DEFAULT_CLI_ARGS",
        ),
        cli_args: corpus_regression_cli_args_from_env_for_test(),
    };
    run_suite("smoke", "regression/baselines/smoke.json", args)
        .unwrap_or_else(|e| panic!("corpus regression smoke case '{case_id}' failed: {e:#}"));
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

    if let Ok(value) =
        std::env::var("EXCEL_SHRINK_CORPUS_REGRESSION_PARALLEL_RANGE_CHUNK_SIZE")
    {
        if !value.trim().is_empty() {
            args.push("--parallel-range-chunk-size".to_string());
            args.push(value.trim().to_string());
        }
    }

    if env_flag_enabled_for_test(
        "EXCEL_SHRINK_CORPUS_REGRESSION_DISABLE_EMPTY_STYLE_REPRESENTATIVES",
    ) {
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
        .map(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

#[cfg(test)]
pub fn smoke_cases_for_corpus_regression_tests(include_slow: bool) -> Vec<String> {
    let suite = load_suite(&repo_root().join("regression/baselines/smoke.json"));
    suite
        .cases
        .into_iter()
        .filter(|case| include_slow || !case.slow.unwrap_or(false))
        .map(|case| case.id)
        .collect()
}

#[cfg(test)]
pub fn available_smoke_cases_for_corpus_regression_tests() -> Vec<String> {
    let suite = load_suite(&repo_root().join("regression/baselines/smoke.json"));
    suite.cases.into_iter().map(|case| case.id).collect()
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
    } else if mode_name == "smoke" && !args.include_slow {
        let slow_cases: Vec<_> = cases
            .iter()
            .filter(|case| case.slow.unwrap_or(false))
            .map(|case| case.id.clone())
            .collect();
        cases.retain(|case| !case.slow.unwrap_or(false));
        if !slow_cases.is_empty() {
            eprintln!(
                "[smoke] Skipping slow cases by default: {}",
                slow_cases.join(", ")
            );
            eprintln!("[smoke] Use --include-slow or --case-id <id> to run them.");
        }
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
            }
            Ok(report) => {
                any_failure = true;
                eprintln!("[{mode_name}] FAIL {}", report.id);
                for failure in &report.failures {
                    eprintln!("  - {failure}");
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
        include_slow: args.include_slow,
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
    let mut analysis_details = Vec::new();

    if output_path.exists() {
        let output_bytes = fs::metadata(&output_path)?.len();
        let ratio = if original_bytes == 0 {
            0.0
        } else {
            (original_bytes as f64 - output_bytes as f64) / original_bytes as f64
        };
        cleaned_bytes = Some(output_bytes);
        reduction_ratio = Some(ratio);

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

    Ok(CaseReport {
        id: case.id.clone(),
        category: case.category.clone(),
        reason: case.reason.clone(),
        dominant_cleanup: case.dominant_cleanup.clone(),
        notes: case.notes.clone(),
        source_repository_url: case.source_repository_url.clone(),
        source_file_url: case.source_file_url.clone(),
        download_url: case.download_url.clone(),
        slow: case.slow.unwrap_or(false),
        input_path: input,
        output_path,
        original_bytes,
        cleaned_bytes,
        reduction_ratio,
        analysis_mode,
        failures,
        analysis_details,
    })
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
        slow: case.slow.unwrap_or(false),
        input_path: repo_root.join(case.relative_input.as_deref().unwrap_or("")),
        output_path: output_root.join(&case.id),
        original_bytes: 0,
        cleaned_bytes: None,
        reduction_ratio: None,
        analysis_mode: false,
        failures: vec![failure],
        analysis_details: Vec::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
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
