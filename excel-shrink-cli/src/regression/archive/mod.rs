//! Bounded, disposable workbook runs. Only reports and difference evidence survive.
use super::*;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::process::{Child, Stdio};
use std::time::Duration;
use zip::ZipArchive;

mod download;
mod study;
mod worker;
mod verified;
pub(super) use study::{run as run_study, StudyArgs};
pub(super) use worker::{run_worker, WorkerArgs};

#[derive(Parser, Debug)]
pub(super) struct ArchiveArgs {
    /// HTTP(S) ZIP URL. Repeat for multiple archives; URLs are not written to reports.
    #[arg(long = "url")]
    urls: Vec<String>,
    /// Local JSON containing urls and an optional access_url for download cookies.
    #[arg(long)]
    request_file: Option<PathBuf>,
    /// Download cache. ZIPs are retained and reused across runs.
    #[arg(long, default_value = ".tmp/archive-regression/downloads")]
    download_root: PathBuf,
    /// Report directory. Reusing it resumes completed entries from its journal.
    #[arg(long)]
    output_root: Option<PathBuf>,
    /// Total time allowed for one workbook, including all selected profiles.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    workbook_timeout_seconds: u64,
    /// Opening Excel or starting the add-in may take this many seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(10..=600))]
    startup_timeout_seconds: u64,
    /// Stop after this many technically completed workbooks in each archive.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    max_success_per_archive: Option<u64>,
    /// Revisit skipped or incomplete entries when resuming a report directory.
    #[arg(long)]
    retry_skipped: bool,
    /// Reject an individual XLSX larger than this before extracting it.
    #[arg(long, default_value_t = 2_147_483_648u64, value_parser = clap::value_parser!(u64).range(1..))]
    max_workbook_bytes: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum JobMode {
    #[default]
    Legacy,
    Prepare,
    Compare,
}

#[derive(Serialize, Deserialize)]
struct MemberJob {
    archive: PathBuf,
    index: usize,
    identity: String,
    name: String,
    work: PathBuf,
    record: PathBuf,
    report_root: PathBuf,
    max_workbook_bytes: u64,
    #[serde(default)]
    mode: JobMode,
    #[serde(default)]
    bundle: Option<PathBuf>,
}

pub(super) fn run(args: ArchiveArgs, log_level: LogLevel) -> Result<()> {
    let root = repo_root();
    let mut request = match args.request_file.as_ref() {
        Some(path) => serde_json::from_slice::<download::Request>(&fs::read(resolve_path(
            &root,
            path.clone(),
        ))?)?,
        None => download::Request::default(),
    };
    request.urls.extend(args.urls.clone());
    ensure!(!request.urls.is_empty(), "pass --url or --request-file");
    ensure!(
        request.urls.iter().collect::<HashSet<_>>().len() == request.urls.len(),
        "duplicate archive URLs"
    );
    let downloads = resolve_path(&root, args.download_root.clone());
    fs::create_dir_all(&downloads)?;
    let output = resolve_path(
        &root,
        args.output_root
            .clone()
            .unwrap_or_else(|| format!(".tmp/regression/archive-{}", timestamp()).into()),
    );
    fs::create_dir_all(&output)?;
    let output = fs::canonicalize(output)?;
    let marker = output.join("archive-run.json");
    let hashes: Vec<_> = request
        .urls
        .iter()
        .map(|u| artifact_cache::sha256(u.as_bytes()))
        .collect();
    let settings = json!({"archives":hashes, "version":1});
    if marker.exists() {
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(&marker)?)? == settings,
            "report directory belongs to a different archive request"
        );
    } else {
        ensure!(
            fs::read_dir(&output)?.next().is_none(),
            "use an empty report directory"
        );
        artifact_cache::atomic_write(&marker, &serde_json::to_vec_pretty(&settings)?)?;
    }
    let run_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(output.join(".run.lock"))?;
    run_lock
        .try_lock()
        .map_err(|e| anyhow::anyhow!("report directory is already in use: {e}"))?;
    let mut records = load_records(&output)?;
    publish(&output, &records, false, false)?;
    if let Err(error) = visual_report::open_visual_report(&output.join("report.html")) {
        eprintln!("Could not open archive report: {error:#}");
    }
    if let Some(url) = &request.access_url {
        download::authorize(&downloads, url)?;
    }
    let mut limited = false;
    for (archive_number, url) in request.urls.iter().enumerate() {
        let label = archive_label(url, archive_number);
        eprintln!("[archive] Downloading or reusing {label}");
        let archive = match download::archive(&downloads, url) {
            Ok(path) => path,
            Err(error) => {
                archive_error(
                    &mut records,
                    &hashes[archive_number],
                    &label,
                    &format!("{error:#}"),
                );
                publish(&output, &records, false, false)?;
                continue;
            }
        };
        let members = match ZipArchive::new(fs::File::open(&archive)?)
            .map_err(anyhow::Error::from)
            .and_then(|mut zip| workbook_members(&mut zip))
        {
            Ok(members) => members,
            Err(error) => {
                archive_error(
                    &mut records,
                    &hashes[archive_number],
                    &label,
                    &format!("read ZIP directory: {error:#}"),
                );
                publish(&output, &records, false, false)?;
                continue;
            }
        };
        if members.is_empty() {
            archive_error(
                &mut records,
                &hashes[archive_number],
                &label,
                "ZIP contains no XLSX entries",
            );
            publish(&output, &records, false, false)?;
            continue;
        }
        records.retain(|r| r["identity"] != format!("archive:{}", hashes[archive_number]));
        eprintln!("[archive] {label}: {} XLSX entries", members.len());
        let mut successes = 0;
        for (position, (index, name)) in members.iter().enumerate() {
            let identity = format!("{}:{index}:{name}", hashes[archive_number]);
            let existing = records.iter().position(|r| r["identity"] == identity);
            if let Some(previous) = existing {
                let complete = records[previous]["complete"] == true;
                let skipped = records[previous]["status"] == "skipped";
                if complete && !(args.retry_skipped && skipped) {
                    successes += u64::from(!skipped);
                    if args
                        .max_success_per_archive
                        .is_some_and(|limit| successes >= limit)
                    {
                        limited |= position + 1 < members.len();
                        break;
                    }
                    continue;
                }
            }
            let key = artifact_cache::sha256(identity.as_bytes());
            let key = &key[..20];
            let work = output.join(".work").join(key);
            // A previous interrupted worker may still own Windows processes.
            worker::cleanup_windows(&work)?;
            if work.exists() {
                fs::remove_dir_all(&work)?;
            }
            let old_evidence = output.join("evidence").join(key);
            if old_evidence.exists() {
                fs::remove_dir_all(old_evidence)?;
            }
            fs::create_dir_all(&work)?;
            fs::create_dir_all(output.join("records"))?;
            let record = output.join("records").join(format!("{key}.json"));
            let job = MemberJob {
                archive: archive.clone(),
                index: *index,
                identity: identity.clone(),
                name: format!("{label} / {name}"),
                work: work.clone(),
                record: record.clone(),
                report_root: output.clone(),
                max_workbook_bytes: args.max_workbook_bytes,
                mode: JobMode::Legacy,
                bundle: None,
            };
            let current = existing.unwrap_or(records.len());
            let pending = json!({"identity":identity,"name":job.name,"complete":false,"status":"in progress","results":[]});
            if current == records.len() {
                records.push(pending);
            } else {
                records[current] = pending;
            }
            write_record(&record, &records[current])?;
            publish(&output, &records, false, false)?;
            let job_path = work.join("job.json");
            fs::write(&job_path, serde_json::to_vec_pretty(&job)?)?;
            eprintln!(
                "[archive] {label} · {}/{} · {name}",
                position + 1,
                members.len()
            );
            let started = Instant::now();
            let outcome = worker::launch(&job, &job_path, &args, log_level, || {
                let progress: Value = serde_json::from_slice(&fs::read(&record)?)?;
                if progress != records[current] {
                    records[current] = progress;
                    publish(&output, &records, false, false)?;
                }
                Ok(())
            });
            let mut result: Value = serde_json::from_slice(&fs::read(&record)?)?;
            if let Err(error) = outcome {
                result["status"] = json!("skipped");
                result["error"] = json!(format!("{error:#}"));
            }
            if let Err(error) = worker::cleanup_windows(&work) {
                result["error"] = json!(format!("process cleanup failed: {error:#}"));
                result["status"] = json!("skipped");
                write_record(&record, &result)?;
                records[current] = result;
                publish(&output, &records, false, false)?;
                return Err(error);
            }
            // Move only referenced PNG evidence. No workbook, history or capture cache survives.
            if let Err(error) =
                worker::retain_evidence(&mut result, &work, &output.join("evidence").join(key))
            {
                result["status"] = json!("skipped");
                result["error"] = json!(format!("preserve report evidence: {error:#}"));
            }
            result["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
            if let Err(error) = fs::remove_dir_all(&work) {
                result["status"] = json!("skipped");
                result["error"] = json!(format!("temporary file cleanup failed: {error}"));
                write_record(&record, &result)?;
                records[current] = result;
                publish(&output, &records, false, false)?;
                anyhow::bail!("cleanup failed; stopping to avoid accumulating workbooks");
            }
            result["complete"] = json!(true);
            write_record(&record, &result)?;
            let skipped = result["status"] == "skipped";
            eprintln!(
                "[archive] {} · {:.1}s",
                result["status"].as_str().unwrap_or("unknown"),
                started.elapsed().as_secs_f64()
            );
            records[current] = result;
            publish(&output, &records, false, false)?;
            successes += u64::from(!skipped);
            if args
                .max_success_per_archive
                .is_some_and(|limit| successes >= limit)
            {
                limited |= position + 1 < members.len();
                break;
            }
        }
    }
    publish(&output, &records, true, limited)?;
    let skipped = records
        .iter()
        .filter(|r| matches!(r["status"].as_str(), Some("skipped" | "archive error")))
        .count();
    let changed = records
        .iter()
        .filter(|r| r["status"] == "differences")
        .count();
    eprintln!("[archive] {} processed · {skipped} skipped/errors · {changed} with differences{}\n[archive] Report: {}",
        records.len(), if limited { " · limited test run" } else { "" }, output.join("report.html").display());
    ensure!(
        skipped == 0 && changed == 0,
        "Archive checks found differences or skipped files; see report"
    );
    Ok(())
}

fn archive_label(url: &str, number: usize) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("archive-{}", number + 1))
}

fn archive_error(records: &mut Vec<Value>, key: &str, name: &str, error: &str) {
    let identity = format!("archive:{key}");
    records.retain(|r| r["identity"] != identity);
    records.push(json!({"identity":identity,"name":name,"complete":true,
        "status":"archive error","error":error,"results":[]}));
}

fn workbook_members<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<Vec<(usize, String)>> {
    let mut members = Vec::new();
    for index in 0..zip.len() {
        // Listing must not decode encrypted or unsupported members. Those are
        // reported by their worker, without preventing the next entry's check.
        let entry = match zip.by_index_raw(index) {
            Ok(entry) => entry,
            Err(_) => {
                // The central directory is readable, but this member's header
                // is not. Let its worker record the error and continue.
                members.push((index, format!("unreadable ZIP entry #{index}")));
                continue;
            }
        };
        if !entry.is_dir()
            && [".xlsx", ".xls"]
                .iter()
                .any(|ext| entry.name().to_ascii_lowercase().ends_with(ext))
        {
            members.push((index, entry.name().to_owned()));
        }
    }
    Ok(members)
}

fn load_records(output: &Path) -> Result<Vec<Value>> {
    let records = output.join("records");
    if !records.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(records)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| {
            let mut record = serde_json::from_slice(&fs::read(p)?)?;
            diagnosis_report::ensure_whitespace_diagnosis(&mut record);
            Ok(record)
        })
        .collect()
}

fn write_record(path: &Path, record: &Value) -> Result<()> {
    let mut record = record.clone();
    diagnosis_report::ensure_whitespace_diagnosis(&mut record);
    artifact_cache::atomic_write(path, &serde_json::to_vec_pretty(&record)?)
}

fn publish(output: &Path, records: &[Value], complete: bool, limited: bool) -> Result<()> {
    let records: Vec<_> = records.iter().cloned().map(|mut record| {
        diagnosis_report::ensure_whitespace_diagnosis(&mut record);
        record
    }).collect();
    let report = json!({"workbooks":records,"total_workbooks":records.len(),"complete":complete,
        "limited":limited,"archive_run":true});
    artifact_cache::atomic_write(
        &output.join("diagnosis.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    verified::write(output, &records)?;
    diagnosis_report::write(output, &report)?;
    Ok(())
}
