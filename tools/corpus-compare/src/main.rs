use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;
use zip::ZipArchive;

#[derive(Parser)]
#[command(
    about = "Shrink every corpus file with HEAD and the working tree, then append comparisons as JSONL"
)]
struct Args {
    #[arg(long)]
    corpus_root: PathBuf,
    #[arg(long)]
    run_root: PathBuf,
    #[arg(long)]
    resume: bool,
    #[arg(long)]
    rebuild: bool,
    #[arg(long = "cli-arg", allow_hyphen_values = true)]
    cli_args: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Input {
    relative_input: String,
    input_path: PathBuf,
    input_bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct BuildCache {
    format_version: u32,
    head_commit: String,
    working_tree_fingerprint: String,
    head_binary: PathBuf,
    working_tree_binary: PathBuf,
}

struct ProcessResult {
    exit_code: i32,
    seconds: f64,
    log: String,
    output: Option<PathBuf>,
}

struct Entry {
    hash: String,
    bytes: usize,
    data: Vec<u8>,
}

struct Worktree {
    repo: PathBuf,
    path: PathBuf,
    active: bool,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        if self.active {
            let _ = Command::new("git")
                .args([
                    "-C",
                    &self.repo.to_string_lossy(),
                    "worktree",
                    "remove",
                    "--force",
                ])
                .arg(&self.path)
                .status();
            let _ = Command::new("git")
                .args(["-C", &self.repo.to_string_lossy(), "worktree", "prune"])
                .status();
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let repo = canonical(command_text(
        Command::new("git").args(["rev-parse", "--show-toplevel"]),
    )?)?;
    let corpus = canonical(&args.corpus_root)?;
    let run_root = absolute_from(&repo, &args.run_root);
    if run_root.exists() && !args.resume && !args.rebuild {
        bail!(
            "run root exists; use --resume, --rebuild, or another path: {}",
            run_root.display()
        );
    }
    fs::create_dir_all(&run_root)?;
    let manifest = discover(&corpus)?;
    if manifest.is_empty() {
        bail!("no XLSX files found below {}", corpus.display());
    }
    fs::write(
        run_root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let head_commit = command_text(Command::new("git").args([
        "-C",
        &repo.to_string_lossy(),
        "rev-parse",
        "HEAD",
    ]))?;
    let fingerprint = working_fingerprint(&repo)?;
    let identity = hash_bytes(
        format!(
            "{head_commit}\n{fingerprint}\n{}\n{:?}",
            corpus.display(),
            args.cli_args
        )
        .as_bytes(),
    );
    let run_id = &identity[..16];
    let report = run_root.join("report.jsonl");
    let completed = if args.resume {
        completed_files(&report, run_id)?
    } else {
        BTreeSet::new()
    };
    append(
        &report,
        &json!({
            "type": if args.resume { "run_resume" } else { "run_start" },
            "run_id": run_id, "timestamp_unix_ms": now_ms(), "head_commit": head_commit,
            "working_tree_fingerprint": fingerprint, "corpus_root": corpus,
            "corpus_files": manifest.len(), "corpus_bytes": manifest.iter().map(|i| i.input_bytes).sum::<u64>(),
            "cli_args": args.cli_args, "already_completed": completed.len(),
        }),
    )?;

    let cache_path = run_root.join("build-cache.json");
    let cache = read_valid_cache(
        &cache_path,
        &head_commit,
        &fingerprint,
        args.resume && !args.rebuild,
    )?;
    let (head_binary, working_binary) = if let Some(cache) = cache {
        println!("Reusing cached HEAD and working-tree binaries.");
        append(
            &report,
            &json!({
                "type": "build_cache_hit", "run_id": run_id, "timestamp_unix_ms": now_ms(),
                "head_binary": cache.head_binary, "working_tree_binary": cache.working_tree_binary,
            }),
        )?;
        (cache.head_binary, cache.working_tree_binary)
    } else {
        let built = build_binaries(
            &repo,
            &run_root,
            &head_commit,
            &fingerprint,
            &report,
            run_id,
        )?;
        fs::write(&cache_path, serde_json::to_vec_pretty(&built)?)?;
        (built.head_binary, built.working_tree_binary)
    };

    let outputs = run_root.join("outputs");
    let logs = run_root.join("logs/files");
    let diffs = run_root.join("diffs");
    fs::create_dir_all(outputs.join("head"))?;
    fs::create_dir_all(outputs.join("working-tree"))?;
    let started = Instant::now();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut processed = 0usize;
    let mut exit = 0;

    for (offset, input) in manifest.iter().enumerate() {
        let index = offset + 1;
        if completed.contains(&input.relative_input) {
            continue;
        }
        println!("[{index}/{}] {}", manifest.len(), input.relative_input);
        let input_hash = hash_file(&input.input_path)?;
        let mut head = shrink(
            "head",
            &head_binary,
            &repo,
            input,
            &outputs.join("head"),
            &logs,
            &args.cli_args,
        )?;
        let mut working = shrink(
            "working-tree",
            &working_binary,
            &repo,
            input,
            &outputs.join("working-tree"),
            &logs,
            &args.cli_args,
        )?;
        let comparison = compare(
            &input.relative_input,
            head.output.take().as_deref(),
            working.output.take().as_deref(),
            &diffs,
            &run_root,
        )?;
        let status = comparison["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        *counts.entry(status.clone()).or_default() += 1;
        if !matches!(status.as_str(), "identical" | "zip_metadata_only") {
            exit = 1;
        }
        append(
            &report,
            &json!({
                "type": "file_comparison", "run_id": run_id, "timestamp_unix_ms": now_ms(),
                "index": index, "total": manifest.len(), "relative_input": input.relative_input,
                "input_bytes": input.input_bytes, "input_sha256": input_hash,
                "head": process_json(&head), "working_tree": process_json(&working),
                "comparison": comparison,
            }),
        )?;
        processed += 1;
        println!(
            "  {status}; head_exit={} working_exit={}",
            head.exit_code, working.exit_code
        );
    }
    append(
        &report,
        &json!({
            "type": "run_end", "run_id": run_id, "timestamp_unix_ms": now_ms(),
            "processed_this_invocation": processed, "completed_total": completed.len() + processed,
            "elapsed_seconds": started.elapsed().as_secs_f64(), "comparison_counts": counts,
            "exit_code": exit,
        }),
    )?;
    println!("Report: {}", report.display());
    if exit != 0 {
        std::process::exit(exit);
    }
    Ok(())
}

fn build_binaries(
    repo: &Path,
    run_root: &Path,
    head_commit: &str,
    fingerprint: &str,
    report: &Path,
    run_id: &str,
) -> Result<BuildCache> {
    let path = run_root.join("build-head-worktree");
    if path.exists() {
        let _ = Command::new("git")
            .args([
                "-C",
                &repo.to_string_lossy(),
                "worktree",
                "remove",
                "--force",
            ])
            .arg(&path)
            .status();
    }
    let mut guard = Worktree {
        repo: repo.to_owned(),
        path: path.clone(),
        active: false,
    };
    ensure(
        Command::new("git")
            .args(["-C", &repo.to_string_lossy(), "worktree", "add", "--detach"])
            .arg(&path)
            .arg("HEAD"),
        "create HEAD worktree",
    )?;
    guard.active = true;
    let head_target = run_root.join("target/head");
    let working_target = run_root.join("target/working-tree");
    let head_log = run_root.join("logs/build-head.log");
    let working_log = run_root.join("logs/build-working-tree.log");
    let head_status = cargo_build(&path, &head_target, &head_log)?;
    append(
        report,
        &json!({
            "type": "build", "run_id": run_id, "timestamp_unix_ms": now_ms(), "revision": "head",
            "exit_code": code(head_status), "log": "logs/build-head.log",
        }),
    )?;
    if !head_status.success() {
        bail!("HEAD build failed; see {}", head_log.display());
    }
    let working_status = cargo_build(repo, &working_target, &working_log)?;
    append(
        report,
        &json!({
            "type": "build", "run_id": run_id, "timestamp_unix_ms": now_ms(), "revision": "working-tree",
            "exit_code": code(working_status), "log": "logs/build-working-tree.log",
        }),
    )?;
    if !working_status.success() {
        bail!("working-tree build failed; see {}", working_log.display());
    }
    let head_binary = cli_binary(&head_target);
    let working_tree_binary = cli_binary(&working_target);
    if !head_binary.is_file() || !working_tree_binary.is_file() {
        bail!("cargo succeeded but a CLI binary is missing");
    }
    Ok(BuildCache {
        format_version: 1,
        head_commit: head_commit.to_owned(),
        working_tree_fingerprint: fingerprint.to_owned(),
        head_binary,
        working_tree_binary,
    })
}

fn cargo_build(source: &Path, target: &Path, log: &Path) -> Result<ExitStatus> {
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(log)?;
    let stderr = file.try_clone()?;
    Ok(Command::new("cargo")
        .args(["build", "--release", "-p", "excel-shrink-cli"])
        .current_dir(source)
        .env("CARGO_TARGET_DIR", target)
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(stderr))
        .status()?)
}

fn read_valid_cache(
    path: &Path,
    head: &str,
    fingerprint: &str,
    allowed: bool,
) -> Result<Option<BuildCache>> {
    if !allowed || !path.is_file() {
        return Ok(None);
    }
    let cache: BuildCache = serde_json::from_slice(&fs::read(path)?)?;
    Ok((cache.format_version == 1
        && cache.head_commit == head
        && cache.working_tree_fingerprint == fingerprint
        && cache.head_binary.is_file()
        && cache.working_tree_binary.is_file())
    .then_some(cache))
}

fn working_fingerprint(repo: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    let diff = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "diff",
            "--binary",
            "HEAD",
            "--",
        ])
        .output()?;
    if !diff.status.success() {
        bail!("git diff failed while fingerprinting the working tree");
    }
    digest.update(b"tracked\0");
    digest.update(diff.stdout);
    let untracked = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()?;
    if !untracked.status.success() {
        bail!("git ls-files failed while fingerprinting");
    }
    let mut paths = untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    paths.sort();
    for relative in paths {
        let path = repo.join(&relative);
        if path.is_file() {
            digest.update(b"untracked\0");
            digest.update(relative.as_bytes());
            digest.update(b"\0");
            let mut file = File::open(path)?;
            let mut buffer = [0u8; 1024 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn discover(root: &Path) -> Result<Vec<Input>> {
    let mut inputs = WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
        })
        .map(|entry| {
            Ok(Input {
                relative_input: entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                input_path: canonical(entry.path())?,
                input_bytes: entry.metadata()?.len(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    inputs.sort_by(|left, right| left.relative_input.cmp(&right.relative_input));
    Ok(inputs)
}

fn shrink(
    label: &str,
    binary: &Path,
    repo: &Path,
    input: &Input,
    output_root: &Path,
    logs: &Path,
    args: &[String],
) -> Result<ProcessResult> {
    let key = case_key(&input.relative_input);
    let output_dir = output_root.join(&key);
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)?;
    }
    fs::create_dir_all(&output_dir)?;
    let log = logs.join(label).join(format!("{key}.log"));
    fs::create_dir_all(log.parent().unwrap())?;
    let file = File::create(&log)?;
    let stderr = file.try_clone()?;
    let started = Instant::now();
    let status = Command::new(binary)
        .arg(&input.input_path)
        .arg(&output_dir)
        .arg("--force-overwrite")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(stderr))
        .status()?;
    let expected = output_dir.join(input.input_path.file_name().context("input has no name")?);
    Ok(ProcessResult {
        exit_code: code(status),
        seconds: started.elapsed().as_secs_f64(),
        log: format!("logs/files/{label}/{key}.log"),
        output: expected.is_file().then_some(expected),
    })
}

fn compare(
    relative: &str,
    head: Option<&Path>,
    working: Option<&Path>,
    diff_root: &Path,
    run_root: &Path,
) -> Result<Value> {
    let (head, working) = match (head, working) {
        (None, None) => return Ok(json!({"status":"both_missing","entry_differences":[]})),
        (None, Some(_)) => return Ok(json!({"status":"head_missing","entry_differences":[]})),
        (Some(_), None) => {
            return Ok(json!({"status":"working_tree_missing","entry_differences":[]}))
        }
        (Some(head), Some(working)) => (head, working),
    };
    let head_hash = hash_file(head)?;
    let working_hash = hash_file(working)?;
    let left = read_entries(head);
    let right = read_entries(working);
    if left.is_err() || right.is_err() {
        return Ok(json!({
            "status":"invalid_output","head_sha256":head_hash,"working_tree_sha256":working_hash,
            "head_error":left.err().map(|e|format!("{e:#}")),
            "working_tree_error":right.err().map(|e|format!("{e:#}")),"entry_differences":[],
        }));
    }
    let left = left.unwrap();
    let right = right.unwrap();
    let names = left
        .keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    for name in names {
        match (left.get(&name), right.get(&name)) {
            (None, Some(_)) => changes.push(json!({"entry":name,"status":"working_tree_only"})),
            (Some(_), None) => changes.push(json!({"entry":name,"status":"head_only"})),
            (Some(a), Some(b)) if a.hash != b.hash => {
                let diff = if text_entry(&name) {
                    write_diff(relative, &name, &a.data, &b.data, diff_root, run_root)?
                } else {
                    None
                };
                changes.push(json!({
                    "entry":name,"status":"changed","head_sha256":a.hash,"working_tree_sha256":b.hash,
                    "head_bytes":a.bytes,"working_tree_bytes":b.bytes,"diff":diff,
                }));
            }
            _ => {}
        }
    }
    let status = if !changes.is_empty() {
        "content_changed"
    } else if head_hash == working_hash {
        "identical"
    } else {
        "zip_metadata_only"
    };
    Ok(json!({
        "status":status,"byte_identical":head_hash==working_hash,"content_identical":changes.is_empty(),
        "head_sha256":head_hash,"working_tree_sha256":working_hash,
        "head_bytes":head.metadata()?.len(),"working_tree_bytes":working.metadata()?.len(),
        "entry_differences":changes,
    }))
}

fn read_entries(path: &Path) -> Result<BTreeMap<String, Entry>> {
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let mut result = BTreeMap::new();
    for index in 0..zip.len() {
        let mut item = zip.by_index(index)?;
        if item.is_dir() {
            continue;
        }
        let name = item.name().to_owned();
        let mut data = Vec::with_capacity(item.size() as usize);
        item.read_to_end(&mut data)?;
        result.insert(
            name,
            Entry {
                hash: hash_bytes(&data),
                bytes: data.len(),
                data,
            },
        );
    }
    Ok(result)
}

fn write_diff(
    relative: &str,
    entry: &str,
    head: &[u8],
    working: &[u8],
    root: &Path,
    run_root: &Path,
) -> Result<Option<String>> {
    if std::str::from_utf8(head).is_err() || std::str::from_utf8(working).is_err() {
        return Ok(None);
    }
    let dir = root.join(case_key(relative));
    fs::create_dir_all(&dir)?;
    let name = safe(entry);
    let a = dir.join(format!("{name}.head"));
    let b = dir.join(format!("{name}.working"));
    let diff = dir.join(format!("{name}.diff"));
    fs::write(&a, head)?;
    fs::write(&b, working)?;
    let output = Command::new("git")
        .args(["diff", "--no-index", "--no-ext-diff", "--text", "--"])
        .arg(&a)
        .arg(&b)
        .output()?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        bail!("git diff failed for {entry}");
    }
    fs::write(&diff, output.stdout)?;
    let _ = fs::remove_file(a);
    let _ = fs::remove_file(b);
    Ok(Some(
        diff.strip_prefix(run_root)?
            .to_string_lossy()
            .replace('\\', "/"),
    ))
}

fn completed_files(report: &Path, run_id: &str) -> Result<BTreeSet<String>> {
    let mut done = BTreeSet::new();
    if !report.is_file() {
        return Ok(done);
    }
    for (number, line) in BufReader::new(File::open(report)?).lines().enumerate() {
        let value: Value = serde_json::from_str(&line?)
            .with_context(|| format!("invalid JSONL at {}:{}", report.display(), number + 1))?;
        if value["type"] == "file_comparison" && value["run_id"] == run_id {
            if let Some(path) = value["relative_input"].as_str() {
                done.insert(path.to_owned());
            }
        }
    }
    Ok(done)
}

fn append(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_data()?;
    Ok(())
}

fn process_json(value: &ProcessResult) -> Value {
    json!({"exit_code":value.exit_code,"elapsed_seconds":value.seconds,"log":value.log})
}
fn cli_binary(target: &Path) -> PathBuf {
    target.join("release").join(if cfg!(windows) {
        "excel-shrink-cli.exe"
    } else {
        "excel-shrink-cli"
    })
}
fn absolute_from(repo: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        repo.join(path)
    }
}
fn canonical(path: impl AsRef<Path>) -> Result<PathBuf> {
    Ok(fs::canonicalize(path.as_ref())
        .with_context(|| format!("path not found: {}", path.as_ref().display()))?)
}
fn code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 1024 * 1024];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn case_key(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("workbook");
    let slug = stem
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}-{}",
        slug.chars().take(70).collect::<String>(),
        &hash_bytes(path.as_bytes())[..12]
    )
}
fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
fn text_entry(path: &str) -> bool {
    path == "[Content_Types].xml"
        || Path::new(path)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "xml" | "rels" | "txt" | "csv"
                )
            })
}
fn command_text(command: &mut Command) -> Result<String> {
    let o = command.output()?;
    if !o.status.success() {
        bail!("command failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(String::from_utf8(o.stdout)?.trim().to_owned())
}
fn ensure(command: &mut Command, action: &str) -> Result<()> {
    let s = command.status()?;
    if !s.success() {
        bail!("failed to {action}: {}", code(s));
    }
    Ok(())
}
