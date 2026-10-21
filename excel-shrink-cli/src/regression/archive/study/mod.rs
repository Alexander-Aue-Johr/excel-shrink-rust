//! Import completed checks into named corpus dashboards.
use super::*;
use super::verified as ledger;

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
pub(in crate::regression) fn run(args: StudyArgs, _log_level: LogLevel) -> Result<()> {
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
    publish(
        &output,
        &records,
        &corpus_info,
        false,
        false,
        "loading archives",
    )?;
    Ok(())
}

fn record_path(output: &Path, id: &str) -> PathBuf {
    output.join("records").join(format!(
        "{}.json",
        &artifact_cache::sha256(id.as_bytes())[..20]
    ))
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
        if row["complete"] == true && row["status"] == "unchanged" {
            row["previously_verified"] = json!(true);
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
