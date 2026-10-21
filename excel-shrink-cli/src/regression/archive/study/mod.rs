//! Separate isolated shrink measurements from targeted Excel comparisons.
use super::*;

mod execution;
use super::verified as ledger;
use execution::process;
mod prepared;

#[derive(Parser, Debug)]
pub(in crate::regression) struct StudyArgs {
    /// Corpus name followed by an HTTP(S) ZIP URL or a local ZIP path. Repeat per corpus.
    #[arg(long,num_args=2,value_names=["NAME","ARCHIVE"])]
    corpus: Vec<String>,
    /// JSON with corpora [{name,url}], or the earlier urls list, and optional access_url.
    #[arg(long)]
    request_file: Option<PathBuf>,
    #[arg(long, default_value = ".tmp/archive-regression/downloads")]
    download_root: PathBuf,
    #[arg(long, default_value = ".tmp/regression/study")]
    output_root: PathBuf,
    /// Import completed unchanged files; previous differences are scheduled first.
    #[arg(long)]
    import_report: Option<PathBuf>,
    /// Import a portable verified-unchanged.csv ledger.
    #[arg(long)]
    verified: Option<PathBuf>,
    /// Import records and rebuild the dashboard without downloading or running Excel.
    #[arg(long)]
    report_only: bool,
    /// Prepare all three profiles without starting visual comparisons.
    #[arg(long, conflicts_with = "compare_only")]
    prepare_only: bool,
    /// Compare already prepared ZIP bundles without shrinking again.
    #[arg(long)]
    compare_only: bool,
    /// Reuse preparations from this executable SHA-256 after reviewing shrink compatibility.
    /// Repeat for each reviewed build; source catalogs and profile settings must still match.
    #[arg(long, value_name = "SHA256", value_parser = preparation_build_hash)]
    reuse_prepared_from: Vec<String>,
    /// Limit new files per corpus, useful for a representative test subset.
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..))]
    limit: Option<u64>,
    /// Retry files previously skipped because a check could not finish.
    #[arg(long)]
    retry_skipped: bool,
    #[arg(long,default_value_t=1800,value_parser=clap::value_parser!(u64).range(1..))]
    workbook_timeout_seconds: u64,
    #[arg(long,default_value_t=120,value_parser=clap::value_parser!(u64).range(10..=600))]
    startup_timeout_seconds: u64,
    #[arg(long,default_value_t=2_147_483_648u64,value_parser=clap::value_parser!(u64).range(1..))]
    max_workbook_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    name: String,
    url: String,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    corpora: Vec<Corpus>,
    #[serde(default)]
    urls: Vec<String>,
    access_url: Option<String>,
}
struct Source {
    id: String,
    name: String,
    path: PathBuf,
    members: Vec<(usize, String)>,
    fingerprint: String,
}

pub(in crate::regression) fn run(args: StudyArgs, log_level: LogLevel) -> Result<()> {
    let root = repo_root();
    let mut request = match &args.request_file {
        Some(path) => {
            serde_json::from_slice::<Request>(&fs::read(resolve_path(&root, path.clone()))?)?
        }
        None => Request::default(),
    };
    request
        .corpora
        .extend(request.urls.iter().enumerate().map(|(i, url)| Corpus {
            name: archive_label(url, i),
            url: url.clone(),
        }));
    request
        .corpora
        .extend(args.corpus.chunks_exact(2).map(|p| Corpus {
            name: p[0].clone(),
            url: p[1].clone(),
        }));
    ensure!(
        !request.corpora.is_empty(),
        "pass --corpus NAME ARCHIVE or --request-file"
    );
    ensure!(
        request
            .corpora
            .iter()
            .map(|c| &c.name)
            .collect::<HashSet<_>>()
            .len()
            == request.corpora.len(),
        "duplicate corpus names"
    );
    let output = resolve_path(&root, args.output_root.clone());
    fs::create_dir_all(&output)?;
    let output = fs::canonicalize(output)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(output.join(".run.lock"))?;
    lock.try_lock()
        .context("report directory is already in use")?;
    fs::create_dir_all(output.join("records"))?;
    let mut records = load_records(&output)?;
    for record in &mut records {
        if record["complete"] != true {
            // A process from the previous invocation is no longer running.
            record["status"] = json!(if record["prepared"] == true {
                "prepared"
            } else {
                "pending"
            });
        }
    }
    let mut corpus_info = Vec::new();
    for corpus in &request.corpora {
        let id = artifact_cache::sha256(corpus.url.as_bytes());
        corpus_info.push(json!({"id":id,"name":corpus.name,"total_workbooks":null}));
    }
    for (i, corpus) in request.corpora.iter().enumerate() {
        let local = if corpus.url.starts_with("http") {
            resolve_path(&root, args.download_root.clone()).join(format!(
                "{}.zip",
                &artifact_cache::sha256(corpus.url.as_bytes())[..20]
            ))
        } else {
            resolve_path(&root, PathBuf::from(&corpus.url))
        };
        if local.is_file() {
            if let Ok(mut zip) = fs::File::open(local)
                .map_err(anyhow::Error::from)
                .and_then(|f| Ok(ZipArchive::new(f)?))
            {
                if let Ok(members) = workbook_members(&mut zip) {
                    corpus_info[i]["total_workbooks"] = json!(members.len());
                }
            }
        }
    }
    for record in &mut records {
        if let Some(corpus) = corpus_info.iter().find(|c| {
            record["identity"]
                .as_str()
                .is_some_and(|id| id.starts_with(&format!("{}:", c["id"].as_str().unwrap())))
        }) {
            record["corpus_id"] = corpus["id"].clone();
            record["corpus_name"] = corpus["name"].clone();
        }
    }
    import(&args, &output, &corpus_info, &mut records)?;
    if args.report_only {
        return publish(
            &output,
            &records,
            &corpus_info,
            false,
            false,
            "loading archives",
        );
    }
    publish_progress(&output, &records, &corpus_info, "loading archives");
    let downloads = resolve_path(&root, args.download_root.clone());
    fs::create_dir_all(&downloads)?;
    if let Some(url) = &request.access_url {
        download::authorize(&downloads, url)?;
    }
    let mut sources = Vec::new();
    for (i, corpus) in request.corpora.iter().enumerate() {
        let id = corpus_info[i]["id"].as_str().unwrap().to_owned();
        let loaded = (|| -> Result<Source> {
            let path = if corpus.url.starts_with("https://") || corpus.url.starts_with("http://") {
                download::archive(&downloads, &corpus.url)?
            } else {
                resolve_path(&root, PathBuf::from(&corpus.url))
            };
            let mut zip = ZipArchive::new(fs::File::open(&path)?)?;
            let members = workbook_members(&mut zip)?;
            let mut catalog = Vec::new();
            for (index, name) in &members {
                let checksum = zip
                    .by_index_raw(*index)
                    .ok()
                    .map(|entry| (entry.crc32(), entry.size()));
                catalog.push((name, checksum));
            }
            let fingerprint = artifact_cache::sha256(&serde_json::to_vec(&catalog)?);
            ensure!(!members.is_empty(), "ZIP contains no XLSX or XLS members");
            Ok(Source {
                id: id.clone(),
                name: corpus.name.clone(),
                path,
                members,
                fingerprint,
            })
        })();
        match loaded {
            Ok(source) => {
                corpus_info[i]["total_workbooks"] = json!(source.members.len());
                sources.push(source);
            }
            Err(error) => {
                archive_error(&mut records, &id, &corpus.name, &format!("{error:#}"));
                let r = records.last_mut().unwrap();
                r["corpus_id"] = json!(id);
                r["corpus_name"] = json!(corpus.name);
                write_record(&record_path(&output, r["identity"].as_str().unwrap()), r)?;
            }
        }
    }
    let limits = ArchiveArgs {
        urls: Vec::new(),
        request_file: None,
        download_root: downloads,
        output_root: Some(output.clone()),
        workbook_timeout_seconds: args.workbook_timeout_seconds,
        startup_timeout_seconds: args.startup_timeout_seconds,
        max_success_per_archive: None,
        retry_skipped: args.retry_skipped,
        max_workbook_bytes: args.max_workbook_bytes,
    };
    let mut limited = false;
    let mut missing_prepared = false;
    let producer = artifact_cache::hash_file(&std::env::current_exe()?)?;
    for source in &sources {
        let mut members = source.members.clone();
        // Previously problematic files go first; untouched entries follow archive order.
        members.sort_by_key(|(index, name)| {
            let id = identity(source, *index, name);
            member_priority(&records, &id)
        });
        let selected: Vec<_> = members
            .into_iter()
            .filter(|(index, name)| {
                let id = identity(source, *index, name);
                records
                    .iter()
                    .find(|r| r["identity"] == id)
                    .is_none_or(|r| {
                        if r["complete"] == true {
                            return args.retry_skipped && r["status"] == "skipped";
                        }
                        true
                    })
            })
            .collect();
        let count = args
            .limit
            .map(|n| n as usize)
            .unwrap_or(selected.len())
            .min(selected.len());
        limited |= count < selected.len();
        let selected = &selected[..count];
        // Save reviewed compatibility before starting any new workbook. A later
        // interruption can then resume normally without repeating the CLI flag.
        if !args.reuse_prepared_from.is_empty() {
            for (index, name) in selected {
                let id = identity(source, *index, name);
                let pos = ensure_record(&mut records, source, *index, name);
                if records[pos]["prepared_by"] != producer
                    && records[pos]["preparation_accepted_by"] != producer
                    && reusable_preparation(
                        &records[pos],
                        &bundle_path(&output, &id),
                        &producer,
                        &source.fingerprint,
                        &args.reuse_prepared_from,
                    )
                {
                    records[pos]["preparation_accepted_by"] = json!(producer);
                    write_record(&record_path(&output, &id), &records[pos])?;
                }
            }
        }
        if !args.compare_only {
            let mut reused = 0;
            for (index, name) in selected {
                let id = identity(source, *index, name);
                let pos = ensure_record(&mut records, source, *index, name);
                let bundle = bundle_path(&output, &id);
                if reusable_preparation(
                    &records[pos],
                    &bundle,
                    &producer,
                    &source.fingerprint,
                    &args.reuse_prepared_from,
                ) {
                    reused += 1;
                    continue;
                }
                records[pos]
                    .as_object_mut()
                    .unwrap()
                    .remove("preparation_accepted_by");
                records[pos]["prepared_by"] = json!(producer);
                records[pos]["source_fingerprint"] = json!(source.fingerprint);
                process(
                    source,
                    *index,
                    &id,
                    pos,
                    JobMode::Prepare,
                    &output,
                    &mut records,
                    &corpus_info,
                    &limits,
                    log_level,
                )?;
            }
            if reused > 0 {
                eprintln!(
                    "[study] {}: reused {reused} prepared workbooks",
                    source.name
                );
            }
        }
        if !args.prepare_only {
            for (index, name) in selected {
                let id = identity(source, *index, name);
                let pos = ensure_record(&mut records, source, *index, name);
                if !reusable_preparation(
                    &records[pos],
                    &bundle_path(&output, &id),
                    &producer,
                    &source.fingerprint,
                    &args.reuse_prepared_from,
                ) {
                    if args.compare_only && records[pos]["status"] != "skipped" {
                        eprintln!("[study] Not prepared with this build: {name}");
                        missing_prepared = true;
                    }
                    continue;
                }
                records[pos]["compared_by"] = json!(producer);
                process(
                    source,
                    *index,
                    &id,
                    pos,
                    JobMode::Compare,
                    &output,
                    &mut records,
                    &corpus_info,
                    &limits,
                    log_level,
                )?;
            }
        }
    }
    let finished = !args.prepare_only && records.iter().all(|r| r["complete"] == true);
    publish(
        &output,
        &records,
        &corpus_info,
        finished,
        limited,
        if args.prepare_only {
            "prepared"
        } else {
            "finished"
        },
    )
    .context("publish final study report; per-workbook records have been saved")?;
    eprintln!("[study] Report: {}", output.join("report.html").display());
    if let Err(error) = visual_report::open_visual_report(&output.join("report.html")) {
        eprintln!("Could not open report: {error:#}");
    }
    ensure!(!missing_prepared, "Some selected workbooks have no preparation from this build; run without --compare-only first");
    ensure!(
        !records.iter().any(|r| matches!(
            r["status"].as_str(),
            Some("differences" | "skipped" | "archive error")
        )),
        "Study contains differences or skipped workbooks; see report"
    );
    Ok(())
}

fn identity(source: &Source, index: usize, name: &str) -> String {
    format!("{}:{index}:{name}", source.id)
}
fn bundle_path(output: &Path, id: &str) -> PathBuf {
    output.join("prepared").join(format!(
        "{}.zip",
        &artifact_cache::sha256(id.as_bytes())[..20]
    ))
}
fn record_path(output: &Path, id: &str) -> PathBuf {
    output.join("records").join(format!(
        "{}.json",
        &artifact_cache::sha256(id.as_bytes())[..20]
    ))
}

fn ensure_record(records: &mut Vec<Value>, source: &Source, index: usize, name: &str) -> usize {
    let id = identity(source, index, name);
    if let Some(i) = records.iter().position(|r| r["identity"] == id) {
        return i;
    }
    records.push(json!({"identity":id,"name":name,"member_name":name,"corpus_id":source.id,"corpus_name":source.name,
        "complete":false,"status":"pending","results":[]}));
    records.len() - 1
}

fn import(
    args: &StudyArgs,
    output: &Path,
    corpora: &[Value],
    records: &mut Vec<Value>,
) -> Result<()> {
    let mut old = Vec::new();
    if let Some(path) = &args.import_report {
        let data: Value = serde_json::from_slice(&fs::read(path)?)?;
        old.extend(
            data["workbooks"]
                .as_array()
                .context("import report has no workbooks")?
                .clone(),
        );
    }
    if let Some(path) = &args.verified {
        old.extend(ledger::read(path)?);
    }
    for mut row in old {
        let Some(id) = row["identity"].as_str().map(str::to_owned) else {
            continue;
        };
        if records.iter().any(|r| r["identity"] == id) {
            continue;
        }
        let Some(corpus) = corpora
            .iter()
            .find(|c| id.starts_with(&format!("{}:", c["id"].as_str().unwrap())))
        else {
            continue;
        };
        row["corpus_id"] = corpus["id"].clone();
        row["corpus_name"] = corpus["name"].clone();
        let member = id.splitn(3, ':').nth(2).unwrap_or("").to_owned();
        row["name"] = json!(member);
        row["member_name"] = json!(member);
        if verified_unchanged(&row) {
            row["previously_verified"] = json!(true);
        } else if row["complete"] == true && row["status"] == "skipped" {
            // Keep earlier technical failures separate until an explicit retry.
            row["previously_skipped"] = json!(true);
            let mut reasons = vec![row["error"]
                .as_str()
                .unwrap_or("Check did not finish")
                .to_owned()];
            for profile in row["results"].as_array().into_iter().flatten() {
                for failure in profile["failures"].as_array().into_iter().flatten() {
                    if let Some(message) = failure.as_str() {
                        reasons.push(format!(
                            "{}: {message}",
                            profile["profile"].as_str().unwrap_or("profile")
                        ));
                    }
                }
            }
            row["error"] = json!(reasons.join("\n"));
            row["results"] = json!([]);
        } else {
            row["previous_difference"] = json!(row["status"] == "differences");
            row["status"] = json!("pending");
            row["complete"] = json!(false);
            row["results"] = json!([]);
            row["error"] = Value::Null;
        }
        write_record(&record_path(output, &id), &row)?;
        records.push(row);
    }
    Ok(())
}

// These files are derived from the durable workbook records. A viewer or virus
// scanner can briefly lock one on Windows; that must not stop a healthy worker.
// Retry at the next changed record or workbook checkpoint, not every polling tick.
fn publish_progress(output: &Path, records: &[Value], corpora: &[Value], phase: &str) {
    if let Err(error) = publish(output, records, corpora, false, false, phase) {
        eprintln!(
            "[study] Could not refresh report in {}; will retry at the next checkpoint: {error:#}",
            output.display()
        );
    }
}

fn publish(
    output: &Path,
    records: &[Value],
    corpora: &[Value],
    complete: bool,
    limited: bool,
    phase: &str,
) -> Result<()> {
    let total = corpora
        .iter()
        .map(|c| c["total_workbooks"].as_u64())
        .collect::<Option<Vec<_>>>()
        .map(|counts| counts.into_iter().sum::<u64>());
    let report = json!({"archive_run":true,"study":true,"workbooks":records,"corpora":corpora,"total_workbooks":total,"complete":complete,"limited":limited,"phase":phase});
    artifact_cache::atomic_write(
        &output.join("diagnosis.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    ledger::write(output, records)?;
    diagnosis_report::write(output, &report).map(|_| ())
}

pub(super) fn run_job(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    match job.mode {
        JobMode::Prepare => prepared::prepare(job, record, log_level),
        JobMode::Compare => prepared::compare(job, record),
        JobMode::Legacy => unreachable!(),
    }
}

#[cfg(test)]
mod tests;

fn member_priority(records: &[Value], identity: &str) -> u8 {
    if records
        .iter()
        .any(|r| r["identity"] == identity && r["previous_difference"] == true)
    {
        0
    } else {
        1
    }
}

fn preparation_build_hash(value: &str) -> std::result::Result<String, String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("expected the full 64-character executable SHA-256".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn reusable_preparation(
    record: &Value,
    bundle: &Path,
    producer: &str,
    source: &str,
    compatible_builds: &[String],
) -> bool {
    record["prepared"] == true
        && bundle.is_file()
        && (record["prepared_by"] == producer
            || record["preparation_accepted_by"] == producer
            || compatible_builds
                .iter()
                .any(|hash| record["prepared_by"] == *hash))
        && record["source_fingerprint"] == source
        && prepared::PROFILES.iter().all(|name| {
            let config = &predefined_visual_profiles()
                .iter()
                .find(|p| p.name == *name)
                .unwrap()
                .config;
            record["results"].as_array().is_some_and(|profiles| {
                profiles.iter().filter(|p| p["profile"] == *name).count() == 1
                    && profiles
                        .iter()
                        .any(|p| p["profile"] == *name && p["config_hash"] == config.config_hash())
            })
        })
}

fn verified_unchanged(record: &Value) -> bool {
    record["complete"] == true
        && record["status"] == "unchanged"
        && record["results"].as_array().is_some_and(|profiles| {
            profiles.iter().any(|p| {
                matches!(
                    p["profile"].as_str(),
                    Some("maximum-cleaning" | "too-aggressive-cleaning")
                ) && p["checks_complete"] == true
                    && p["status"] == "visually unchanged"
                    && p["failures"]
                        .as_array()
                        .is_none_or(|failures| failures.is_empty())
            })
        })
}
