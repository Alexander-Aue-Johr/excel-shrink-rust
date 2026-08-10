use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    corpus_root: PathBuf,
    #[arg(long)]
    comparison_run_root: PathBuf,
    #[arg(long)]
    benchmark_root: PathBuf,
    #[arg(long = "input", required = true)]
    inputs: Vec<String>,
    #[arg(long, default_value_t = 1)]
    warmups: usize,
    #[arg(long, default_value_t = 2)]
    measured_rounds: usize,
}

#[derive(Deserialize)]
struct BuildCache {
    head_commit: String,
    working_tree_fingerprint: String,
    head_binary: PathBuf,
    working_tree_binary: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.measured_rounds == 0 || args.measured_rounds % 2 != 0 {
        bail!("--measured-rounds must be a positive even number");
    }
    fs::create_dir_all(&args.benchmark_root)?;
    let report = args.benchmark_root.join("benchmark.jsonl");
    let cache: BuildCache = serde_json::from_slice(&fs::read(
        args.comparison_run_root.join("build-cache.json"),
    )?)?;
    for binary in [&cache.head_binary, &cache.working_tree_binary] {
        if !binary.is_file() {
            bail!("cached binary is missing: {}", binary.display());
        }
    }
    append(
        &report,
        &json!({
            "type":"benchmark_start", "timestamp_unix_ms":now_ms(),
            "head_commit":cache.head_commit,
            "working_tree_fingerprint":cache.working_tree_fingerprint,
            "warmups":args.warmups, "measured_rounds":args.measured_rounds,
            "inputs":args.inputs,
        }),
    )?;

    for (case_index, relative) in args.inputs.iter().enumerate() {
        let input = args.corpus_root.join(relative);
        if !input.is_file() {
            bail!("input is missing: {}", input.display());
        }
        println!("[{}/{}] {relative}", case_index + 1, args.inputs.len());
        for round in 0..(args.warmups + args.measured_rounds) {
            let measured = round >= args.warmups;
            let measured_index = round.saturating_sub(args.warmups);
            let working_first = measured && measured_index % 2 == 1;
            let order = if working_first {
                [
                    ("working_tree", &cache.working_tree_binary),
                    ("head", &cache.head_binary),
                ]
            } else {
                [
                    ("head", &cache.head_binary),
                    ("working_tree", &cache.working_tree_binary),
                ]
            };
            for (position, (revision, binary)) in order.into_iter().enumerate() {
                let output = args
                    .benchmark_root
                    .join("outputs")
                    .join(format!("case-{case_index}-round-{round}-{revision}"));
                fs::create_dir_all(&output)?;
                let started = Instant::now();
                let status = Command::new(binary)
                    .arg(&input)
                    .arg(&output)
                    .arg("--force-overwrite")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .with_context(|| format!("failed to run {}", binary.display()))?;
                let seconds = started.elapsed().as_secs_f64();
                append(
                    &report,
                    &json!({
                        "type":"measurement", "timestamp_unix_ms":now_ms(),
                        "input":relative, "case_index":case_index,
                        "round":round, "measured":measured,
                        "order_position":position + 1, "revision":revision,
                        "elapsed_seconds":seconds, "exit_code":status.code(),
                    }),
                )?;
                println!(
                    "  round {round} {revision}: {seconds:.3}s{}",
                    if measured { "" } else { " (warm-up)" }
                );
                if !status.success() {
                    bail!("{revision} failed for {relative}");
                }
            }
        }
    }
    append(
        &report,
        &json!({"type":"benchmark_end","timestamp_unix_ms":now_ms()}),
    )?;
    println!("Report: {}", report.display());
    Ok(())
}

fn append(path: &Path, value: &Value) -> Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
