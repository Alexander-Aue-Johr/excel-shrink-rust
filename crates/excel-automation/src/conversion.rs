//! XLS-to-XLSX conversion and XLSX save copies through the shared owned Excel pool.
use crate::{context::RunContext, pool::performance};
use anyhow::{ensure, Context, Result};
use corpus_runner::{artifacts, sources::Input};
use std::path::PathBuf;

pub struct Options {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub workers: usize,
    pub recursive: bool,
}

pub fn run(options: Options, context: RunContext) -> Result<()> {
    let inputs = collect_inputs(&options)?;
    performance::run_conversion(inputs, options.output, options.workers, context)
}

/// Rebuild dependency trees and save copies without modifying the source files.
pub fn rebuild(options: Options, context: RunContext) -> Result<()> {
    let inputs = collect_inputs(&options)?;
    performance::run_rebuild(inputs, options.output, options.workers, context)
}

fn collect_inputs(options: &Options) -> Result<Vec<Input>> {
    ensure!(
        !options.inputs.is_empty(),
        "Choose at least one XLS or XLSX input"
    );
    ensure!(options.workers > 0, "Conversion workers must be positive");
    let paths =
        corpus_runner::sources::expand_paths(&options.inputs, &["xls", "xlsx"], options.recursive)?;
    let inputs = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            ensure!(
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xls")
                        || extension.eq_ignore_ascii_case("xlsx")),
                "Conversion inputs must use XLS or XLSX"
            );
            let path = std::path::absolute(path)?;
            let path = match std::fs::canonicalize(&path) {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => path,
                Err(error) => return Err(error.into()),
            };
            let identity = artifacts::sha256(path.to_string_lossy().as_bytes());
            Ok(Input {
                source_id: format!("explicit:{index}:{identity}"),
                name: path
                    .file_name()
                    .context("Input has no filename")?
                    .to_string_lossy()
                    .into_owned(),
                baseline: path,
                expected_sha256: None,
                raw_source: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(inputs)
}
