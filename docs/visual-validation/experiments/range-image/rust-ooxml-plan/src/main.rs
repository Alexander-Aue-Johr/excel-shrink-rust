mod visual_plan;
mod visual_xml;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    collections::HashSet,
    env,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
struct PlanDocument {
    schema: &'static str,
    generated_unix_ms: u128,
    total_rust_plan_ms: f64,
    workbooks: Vec<visual_plan::WorkbookVisualPlan>,
}

fn parse_args() -> Result<(PathBuf, PathBuf)> {
    let mut args = env::args().skip(1);
    let mut file_list = None;
    let mut output = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--file-list" => {
                ensure!(file_list.is_none(), "--file-list was provided more than once");
                file_list = Some(PathBuf::from(
                    args.next().context("--file-list requires a path")?,
                ));
            }
            "--output" => {
                ensure!(output.is_none(), "--output was provided more than once");
                output = Some(PathBuf::from(
                    args.next().context("--output requires a path")?,
                ));
            }
            "-h" | "--help" => {
                println!(
                    "Usage: visual-validation-ooxml-plan --file-list PATH --output PATH"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok((
        file_list.context("missing --file-list")?,
        output.context("missing --output")?,
    ))
}

fn normalize_wsl_input_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in ["\\\\wsl.localhost\\", "\\\\wsl$\\"] {
        if lower.starts_with(&prefix.to_ascii_lowercase()) {
            let remainder = &trimmed[prefix.len()..];
            if let Some(separator) = remainder.find('\\') {
                let linux = remainder[separator + 1..].replace('\\', "/");
                return PathBuf::from(format!("/{linux}"));
            }
        }
    }
    PathBuf::from(trimmed)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("visual-plan.json");
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .with_context(|| format!("write temporary plan {}", temporary.display()))?;
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("replace old plan {}", path.display()))?;
    }
    fs::rename(&temporary, path)
        .with_context(|| format!("publish visual plan {}", path.display()))?;
    Ok(())
}

fn main() -> Result<()> {
    let (file_list, output) = parse_args()?;
    let text = fs::read_to_string(&file_list)
        .with_context(|| format!("read file list {}", file_list.display()))?;

    let mut workbooks = Vec::new();
    let mut names = HashSet::new();
    let mut total_rust_plan_ms = 0.0;

    for raw in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let path = normalize_wsl_input_path(raw);
        ensure!(
            path.is_file(),
            "workbook from file list does not exist in WSL: {} (source entry: {raw})",
            path.display()
        );
        let plan = visual_plan::build_workbook_visual_plan(&path)?;
        ensure!(
            names.insert(plan.workbook.to_ascii_lowercase()),
            "duplicate workbook file name in visual plan: {}",
            plan.workbook
        );
        total_rust_plan_ms += plan.rust_plan_ms;
        workbooks.push(plan);
    }

    ensure!(!workbooks.is_empty(), "visual-plan file list is empty");

    let document = PlanDocument {
        schema: "excel-shrink.visual-validation.strategy-f.v2",
        generated_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        total_rust_plan_ms,
        workbooks,
    };
    let bytes = serde_json::to_vec_pretty(&document)?;
    let mut with_newline = bytes;
    with_newline.push(b'\n');
    write_atomic(&output, &with_newline)?;
    println!("{}", output.display());
    Ok(())
}
