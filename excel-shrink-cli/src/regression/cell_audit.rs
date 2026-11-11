//! Compare saved cell contents with the existing byte-view comparator.
use super::{artifact_cache, formula_trace, office_plan, xlsx_value_compare};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::File, path::PathBuf, time::Instant};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(
        long,
        required_unless_present = "manifest",
        conflicts_with = "manifest"
    )]
    left: Option<PathBuf>,
    #[arg(
        long,
        required_unless_present = "manifest",
        conflicts_with = "manifest"
    )]
    right: Option<PathBuf>,
    /// JSON array of objects containing label, left and right paths.
    #[arg(long, conflicts_with_all = ["left", "right"])]
    manifest: Option<PathBuf>,
    /// Maximum changed-cell records examined for diagnostic dependency paths.
    #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=65_536))]
    formula_sample_limit: u32,
    /// A new directory for per-pair evidence and the summary.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Deserialize)]
struct Pair {
    #[serde(default)]
    label: String,
    left: PathBuf,
    right: PathBuf,
}

pub fn run(args: Args) -> Result<()> {
    let pairs = match args.manifest {
        Some(path) => serde_json::from_reader(File::open(path)?)?,
        None => vec![Pair {
            label: "explicit pair".into(),
            left: args.left.context("Missing left workbook")?,
            right: args.right.context("Missing right workbook")?,
        }],
    };
    ensure!(!pairs.is_empty(), "The cell audit selection is empty");
    std::fs::create_dir(&args.output).context("Choose a new audit output directory")?;
    let mut summary = Vec::new();
    let mut failures = 0;
    for (index, pair) in pairs.iter().enumerate() {
        let started = Instant::now();
        let result = inspect(pair, args.formula_sample_limit as usize);
        let record = match result {
            Ok(report) => report,
            Err(error) => json!({"status":"failed", "error":format!("{error:#}")}),
        };
        if record["status"] == "failed" {
            failures += 1;
        }
        let file = format!("pair-{index:06}.json");
        artifact_cache::atomic_write(&args.output.join(&file), &serde_json::to_vec(&record)?)?;
        summary.push(json!({
            "label":pair.label, "report":file, "status":record["status"],
            "difference_records":record["difference_records"],
            "elapsed_seconds":started.elapsed().as_secs_f64(),
        }));
        artifact_cache::atomic_write(
            &args.output.join("progress.json"),
            &serde_json::to_vec(&json!({
                "schema":1,"total":pairs.len(),"completed":index + 1,"failed":failures,
            }))?,
        )?;
    }
    artifact_cache::atomic_write(
        &args.output.join("summary.json"),
        &serde_json::to_vec(&json!({"schema":1,"total":pairs.len(),"pairs":summary}))?,
    )?;
    println!("Cell audit reports: {}", args.output.display());
    ensure!(
        failures == 0,
        "{failures} cell audit pairs failed; inspect summary.json"
    );
    Ok(())
}

fn inspect(pair: &Pair, sample_limit: usize) -> Result<Value> {
    let left_hash = artifact_cache::hash_file(&pair.left)?;
    let right_hash = artifact_cache::hash_file(&pair.right)?;
    let mut left_archive = zip::ZipArchive::new(File::open(&pair.left)?)?;
    let mut right_archive = zip::ZipArchive::new(File::open(&pair.right)?)?;
    ensure!(
        office_plan::formula_worksheet_parts(&mut left_archive)?
            == office_plan::formula_worksheet_parts(&mut right_archive)?,
        "Worksheet names, order or part mappings changed; explicit correspondence is required"
    );
    let report = xlsx_value_compare::compare_workbook_cell_contents(&pair.left, &pair.right)?;
    let diagnostics = if report.differences.is_empty() {
        json!({"status":"not_needed"})
    } else {
        explain(pair, &report, sample_limit)
            .unwrap_or_else(|error| json!({"status":"unavailable", "reason":format!("{error:#}")}))
    };
    ensure!(
        left_hash == artifact_cache::hash_file(&pair.left)?
            && right_hash == artifact_cache::hash_file(&pair.right)?,
        "A comparison input changed during the audit"
    );
    Ok(json!({
        "schema":1, "label":pair.label,
        "left_sha256":left_hash, "right_sha256":right_hash,
        "status":if report.differences.is_empty() {"equal_cell_contents"} else {"differences_found"},
        "difference_records":report.difference_count,
        "comparison":report, "formula_diagnostics":diagnostics,
        "policy":"Strict saved values and formula preservation. Empty layout-only cells are ignored. Diagnostic dependency paths do not suppress differences or prove exclusive causation.",
    }))
}

fn explain(
    pair: &Pair,
    report: &xlsx_value_compare::WorkbookComparisonReport,
    sample_limit: usize,
) -> Result<Value> {
    let mut archive = zip::ZipArchive::new(File::open(&pair.left)?)?;
    let names: BTreeMap<_, _> = office_plan::formula_worksheet_parts(&mut archive)?
        .into_iter()
        .map(|(name, part)| (part, name))
        .collect();
    let targets = report
        .differences
        .iter()
        .take(sample_limit)
        .map(|difference| {
            let sheet = names
                .get(&difference.sheet_file)
                .context("Unknown worksheet identity")?;
            Ok((sheet.clone(), difference.cell_ref.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    let left = formula_trace::read_for_cells(&pair.left, &targets)?;
    let right = formula_trace::read_for_cells(&pair.right, &targets)?;
    let mut examples = Vec::new();
    for difference in report.differences.iter().take(sample_limit) {
        let sheet = names
            .get(&difference.sheet_file)
            .context("Unknown worksheet identity")?;
        examples.push(formula_trace::compare(
            &left,
            &right,
            sheet,
            &difference.cell_ref,
        )?);
    }
    Ok(json!({
        "status":"diagnostic_samples", "examples":examples,
        "sample_limit":sample_limit, "difference_records":report.difference_count,
        "all_records_examined":report.difference_count <= sample_limit
            && report.differences.len() == report.difference_count,
        "all_differences_explained":false,
    }))
}
