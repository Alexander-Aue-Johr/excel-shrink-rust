//! Separate isolated shrink measurements from targeted Excel comparisons.
use super::*;

mod control_room;
mod execution;
mod fallback;
mod preparation;
mod preparation_slots;
mod progress;
mod startup;
use super::verified as ledger;
use execution::process;
mod prepared;
mod provenance;
mod scheduler;
mod reduction_order;
mod selection;
mod storage_guard;
mod strategy;
mod transfer;
mod whitespace;

pub(super) fn terminate_requested(root: &Path) -> bool {
    control_room::terminate_requested(root)
}

#[derive(Parser, Debug)]
pub(in crate::regression) struct StudyArgs {
    /// Corpus name followed by an HTTP(S) ZIP URL or a local ZIP path. Repeat per corpus.
    #[arg(long,num_args=2,value_names=["NAME","ARCHIVE"])]
    corpus: Vec<String>,
    /// Execute only this corpus name; repeat to select several. All corpora stay in the report.
    #[arg(long, value_name = "NAME")]
    run_corpus: Vec<String>,
    /// Size-ranked part K/N of each selected corpus; 1/N contains the smallest files.
    #[arg(long, value_name = "K/N")]
    size_part: Option<selection::SizePart>,
    /// Processing order within the selected partition; completed files remain skipped.
    #[arg(long, value_enum, default_value = "priority")]
    order: selection::ProcessingOrder,
    /// Write an additional copy of the deterministic full selection manifest.
    #[arg(long, value_name = "PATH")]
    selection_manifest: Option<PathBuf>,
    /// Compare only workbook identities in this JSON array, in the listed order.
    #[arg(long, value_name = "JSON")]
    workbook_list: Option<PathBuf>,
    /// Compare the strongest relative Default reductions first, after preparation.
    #[arg(long)]
    largest_reduction_first: bool,
    /// Write catalogs and selection manifests without shrinking or starting Excel.
    #[arg(long, conflicts_with_all = ["report_only", "prepare_only", "compare_only", "diagnose_whitespace_only"])]
    selection_only: bool,
    /// JSON with corpora [{name,url}], or the earlier urls list, and optional access_url.
    #[arg(long)]
    request_file: Option<PathBuf>,
    #[arg(long, default_value = ".tmp/archive-regression/downloads")]
    download_root: PathBuf,
    #[arg(long, default_value = ".tmp/regression/study")]
    output_root: PathBuf,
    /// Human-readable machine alias; hardware identity is collected independently.
    #[arg(long, value_name = "NAME")]
    machine_label: Option<String>,
    /// Merge a verified result ZIP, retaining conflicting records for review. Repeat as needed.
    #[arg(long, value_name = "ZIP")]
    import_results: Vec<PathBuf>,
    /// Export completed records, evidence and machine manifests after this invocation.
    #[arg(long, value_name = "ZIP")]
    export_results: Option<PathBuf>,
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
    /// Revisit the full scans deferred by CRC/length-based edge captures.
    /// Requires reusable preparation bundles from an earlier study run.
    #[arg(long, requires = "compare_only", conflicts_with = "report_only")]
    full_scan_deferred: bool,
    /// Keep original and shrink variants open for visual comparisons.
    #[arg(long)]
    resident_capture: bool,
    /// Compare Original and Default before and after an in-place Default rebuild.
    #[arg(long, requires = "resident_capture", conflicts_with_all = ["aggressive_first", "preparation_hash_profile", "full_scan_deferred", "diagnose_whitespace_only", "adaptive_concurrency"])]
    targeted_comparison: bool,
    /// Omit Check Performance from the targeted workflow; compare Default only.
    #[arg(long, requires = "targeted_comparison")]
    targeted_skip_check_performance: bool,
    /// Include native Check Performance in targeted comparisons (off by default).
    #[arg(long, requires = "targeted_comparison", conflicts_with = "targeted_skip_check_performance")]
    targeted_check_performance: bool,
    /// Exclude charts only with proven unchanged dependencies, or capture every chart.
    #[arg(long, value_enum, default_value_t = crate::regression::targeted_comparison::ChartCaptureMode::Deterministic)]
    chart_capture_mode: crate::regression::targeted_comparison::ChartCaptureMode,
    /// Retain verified-equal image evidence; false keeps integrity receipts and differing images.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    retain_equal_images: bool,
    /// Screen the selection with the aggressive profile before full follow-up.
    #[arg(long, requires = "resident_capture", conflicts_with_all = ["prepare_only", "diagnose_whitespace_only", "full_scan_deferred"])]
    aggressive_first: bool,
    /// Analyze selected capture ranges twice and retain differences without a full visual follow-up.
    #[arg(long, value_parser = ["default", "maximum-cleaning", "too-aggressive-cleaning"],
        requires = "resident_capture",
        conflicts_with_all = ["aggressive_first", "full_scan_deferred", "diagnose_whitespace_only"])]
    preparation_hash_profile: Option<String>,
    /// Hide Excel comparison windows from startup while keeping the Office add-in active.
    #[arg(long)]
    hide_excel: bool,
    /// Maximum concurrent comparisons: 8 in resident mode, 1 otherwise.
    #[arg(
        long,
        default_value_t = 1,
        default_value_if("resident_capture", "true", "8"),
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(0..)
    )]
    comparison_workers: usize,
    /// Add groups as primary clients become ready and live memory budgets permit.
    #[arg(long, requires = "resident_capture")]
    adaptive_concurrency: bool,
    /// Serve the live control room; manually set concurrent groups and capture tile images in step.
    #[arg(long, requires = "resident_capture", conflicts_with = "adaptive_concurrency")]
    control_room: bool,
    /// Loopback port for the live control room.
    #[arg(long, default_value_t = 8771, value_parser = clap::value_parser!(u16).range(1024..))]
    control_room_port: u16,
    /// Diagnose completed differences without repeating the three primary comparisons.
    #[arg(long, conflicts_with_all = ["prepare_only", "compare_only"])]
    diagnose_whitespace_only: bool,
    /// Reuse preparations from this executable SHA-256 after reviewing shrink compatibility.
    /// Repeat for each reviewed build; source catalogs and profile settings must still match.
    #[arg(long, value_name = "SHA256", value_parser = preparation_build_hash)]
    reuse_prepared_from: Vec<String>,
    /// Limit new files per corpus, useful for a representative test subset.
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..))]
    limit: Option<u64>,
    /// Retry previously skipped files once in this invocation (the default).
    #[arg(
        long,
        default_value_t = true,
        action = clap::ArgAction::SetTrue,
        conflicts_with = "no_retry_skipped"
    )]
    retry_skipped: bool,
    /// Leave previously skipped files complete instead of retrying them.
    #[arg(long, conflicts_with = "retry_skipped")]
    no_retry_skipped: bool,
    #[arg(long,default_value_t=1800,value_parser=clap::value_parser!(u64).range(1..))]
    workbook_timeout_seconds: u64,
    #[arg(long,default_value_t=120,value_parser=clap::value_parser!(u64).range(10..=600))]
    startup_timeout_seconds: u64,
    #[arg(long,default_value_t=2_147_483_648u64,value_parser=clap::value_parser!(u64).range(1..))]
    max_workbook_bytes: u64,
}

impl StudyArgs {
    fn preparation_profile(&self) -> Option<&str> {
        if self.targeted_comparison { Some("default") } else { self.preparation_hash_profile.as_deref() }
    }
    fn should_retry_skipped(&self) -> bool {
        self.retry_skipped && !self.no_retry_skipped
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    name: String,
    url: String,
    /// Stable identity when the same archive has different local paths on other machines.
    #[serde(default)]
    id: Option<String>,
    /// Optional known total for offline corpora; an available archive takes precedence.
    #[serde(default)]
    total_workbooks: Option<u64>,
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
    selection: selection::Plan,
}

fn has_deferred_full_scan(record: &Value) -> bool {
    record["capture_deferred"]
        .as_object()
        .is_some_and(|profiles| {
            profiles.values().any(|sheets| {
                sheets.as_array().is_some_and(|indices| !indices.is_empty())
            })
        })
}

pub(in crate::regression) fn run(mut args: StudyArgs, log_level: LogLevel) -> Result<()> {
    args.targeted_skip_check_performance = !args.targeted_check_performance;
    match run_inner(args, log_level) {
        Err(error) if error.is::<control_room::StopRequested>() => {
            eprintln!("[study] Run stopped. Results saved; the runner can now be updated.");
            Ok(())
        }
        result => result,
    }
}

fn run_inner(args: StudyArgs, log_level: LogLevel) -> Result<()> {
    ensure!(
        !args.full_scan_deferred || args.resident_capture,
        "--full-scan-deferred requires --resident-capture"
    );
    ensure!(
        args.comparison_workers > 0 || args.control_room,
        "zero comparison groups require --control-room so the run can be resumed"
    );
    ensure!(
        args.resident_capture || args.comparison_workers == 1,
        "parallel workbook comparisons require --resident-capture"
    );
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
            id: None,
            total_workbooks: None,
        }));
    request
        .corpora
        .extend(args.corpus.chunks_exact(2).map(|p| Corpus {
            name: p[0].clone(),
            url: p[1].clone(),
            id: None,
            total_workbooks: None,
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
    let active = selection::active_corpora(&request.corpora, &args.run_corpus)?;
    let ids = request
        .corpora
        .iter()
        .map(corpus_id)
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        ids.iter().collect::<HashSet<_>>().len() == ids.len(),
        "duplicate corpus IDs"
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
    // Serve the dashboard before scanning large ZIP catalogs or loading history.
    let mut room = if args.control_room && !args.report_only && !args.selection_only {
        let startup = json!({"run_id":provenance::new_uuid()?});
        let room = control_room::Room::start(&output, &args, &startup)?;
        room.set_run(&startup, "loading archives")?;
        Some(room)
    } else {
        None
    };
    let mut startup = startup::Startup::new(&output, room.is_some());
    startup.update("loading_records", "", 0, None)?;
    fs::create_dir_all(output.join("records"))?;
    let mut records = load_records_with_progress(&output, |done, total| startup.update("loading_records", "", done, Some(total)))?;
    for record in &mut records {
        if !args.report_only && !args.selection_only {
            record["targeted_check_performance_required"] = json!(args.targeted_comparison && !args.targeted_skip_check_performance);
        }
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
    let mut expected_sources: std::collections::BTreeMap<String, String> =
        ids.iter().map(|id| (id.clone(), String::new())).collect();
    for (i, corpus) in request.corpora.iter().enumerate() {
        corpus_info.push(
            json!({"id":ids[i],"name":corpus.name,"total_workbooks":corpus.total_workbooks,
            "selected":active[i], "size_part":if active[i] { args.size_part } else { None }}),
        );
        if let Ok(data) = fs::read(
            output
                .join("source-catalogs")
                .join(format!("{}.json", ids[i])),
        ) {
            if let Ok(catalog) = serde_json::from_slice::<Value>(&data) {
                if catalog["corpus_id"] == ids[i] {
                    corpus_info[i]["total_workbooks"] = catalog["total_workbooks"].clone();
                    if let Some(fingerprint) = catalog["source_fingerprint"].as_str() {
                        expected_sources.insert(ids[i].clone(), fingerprint.to_owned());
                    }
                }
            }
        }
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
            startup.update("opening_archive", &corpus.name, 0, None)?;
            if let Ok(mut zip) = fs::File::open(local)
                .map_err(anyhow::Error::from)
                .and_then(|f| Ok(ZipArchive::new(std::io::BufReader::new(f))?))
            {
                if let Ok(members) = workbook_members_with_progress(&mut zip, |done, total| startup.update("scanning_archive", &corpus.name, done, Some(total))) {
                    corpus_info[i]["total_workbooks"] = json!(members.len());
                    let catalog: Vec<_> = members
                        .iter()
                        .enumerate()
                        .map(|(position, (index, name))| {
                            let checksum = zip
                                .by_index_raw(*index)
                                .ok()
                                .map(|entry| (entry.crc32(), entry.size()));
                            startup.update("checking_catalog", &corpus.name, position + 1, Some(members.len()))?;
                            Ok((name, checksum))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    expected_sources.insert(
                        ids[i].clone(),
                        artifact_cache::sha256(&serde_json::to_vec(&catalog)?),
                    );
                }
            }
        }
    }
    for package in &args.import_results {
        startup.update("importing_results", "", 0, None)?;
        let receipt = transfer::import_package_locked(
            &output, &resolve_path(&root, package.clone()), &expected_sources, &lock,
        )?;
        eprintln!("[study] Result import: {receipt}");
    }
    if !args.import_results.is_empty() {
        records = load_records(&output)?;
        for record in &mut records {
            if record["complete"] != true {
                record["status"] = json!(if record["prepared"] == true { "prepared" } else { "pending" });
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
    startup.update("importing_results", "", 0, None)?;
    import(&args, &output, &corpus_info, &mut records)?;
    if args.report_only {
        publish(
            &output,
            &records,
            &corpus_info,
            selection::complete(&corpus_info, &records),
            false,
            "loading archives",
        )?;
        return export_results(&args, &root, &output, &lock);
    }
    publish_progress(&output, &records, &corpus_info, "loading archives");
    let downloads = resolve_path(&root, args.download_root.clone());
    fs::create_dir_all(&downloads)?;
    let needs_download = request.corpora.iter().enumerate().any(|(i, c)| {
        active[i]
            && (c.url.starts_with("https://") || c.url.starts_with("http://"))
            && !downloads
                .join(format!(
                    "{}.zip",
                    &artifact_cache::sha256(c.url.as_bytes())[..20]
                ))
                .is_file()
    });
    if needs_download {
        if let Some(url) = &request.access_url {
            download::authorize(&downloads, url)?;
        }
    }
    let mut sources = Vec::new();
    for (i, corpus) in request.corpora.iter().enumerate() {
        if !active[i] {
            continue;
        }
        let id = corpus_info[i]["id"].as_str().unwrap().to_owned();
        startup.update("opening_archive", &corpus.name, 0, None)?;
        let loaded = (|| -> Result<Source> {
            let path = if corpus.url.starts_with("https://") || corpus.url.starts_with("http://") {
                download::archive(&downloads, &corpus.url)?
            } else {
                resolve_path(&root, PathBuf::from(&corpus.url))
            };
            let mut zip = ZipArchive::new(std::io::BufReader::new(fs::File::open(&path)?))?;
            let members = workbook_members_with_progress(&mut zip, |done, total| startup.update("scanning_archive", &corpus.name, done, Some(total)))?;
            let mut catalog = Vec::new();
            let mut sized = Vec::new();
            for (position, (index, name)) in members.iter().enumerate() {
                let checksum = zip
                    .by_index_raw(*index)
                    .ok()
                    .map(|entry| (entry.crc32(), entry.size()));
                sized.push(selection::CatalogMember {
                    archive_index: *index,
                    member_name: name.clone(),
                    bytes: checksum.map(|(_, size)| size),
                    crc32: checksum.map(|(crc, _)| crc),
                });
                catalog.push((name, checksum));
                startup.update("selecting_partition", &corpus.name, position + 1, Some(members.len()))?;
            }
            let fingerprint = artifact_cache::sha256(&serde_json::to_vec(&catalog)?);
            ensure!(!members.is_empty(), "ZIP contains no XLSX or XLS members");
            Ok(Source {
                id: id.clone(),
                name: corpus.name.clone(),
                path,
                members,
                fingerprint,
                selection: selection::Plan::new(sized, args.size_part)?,
            })
        })();
        match loaded {
            Ok(source) => {
                corpus_info[i]["total_workbooks"] = json!(source.members.len());
                corpus_info[i]["selected_workbooks"] = json!(source.selection.selected_workbooks);
                fs::create_dir_all(output.join("source-catalogs"))?;
                artifact_cache::atomic_write(
                    &output
                        .join("source-catalogs")
                        .join(format!("{}.json", source.id)),
                    &serde_json::to_vec_pretty(&json!({"version":1,"corpus_id":source.id,
                        "corpus_name":source.name,"source_fingerprint":source.fingerprint,
                        "total_workbooks":source.members.len(),"catalog_sha256":source.selection.catalog_sha256}))?,
                )?;
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
    let workbook_list = selection::read_workbook_list(args.workbook_list.as_deref(), &sources)?;
    let manifest = json!({"version":1,"algorithm":"uncompressed-bytes-name-index-v1", "processing_order":args.order,
        "workbook_list":workbook_list,
        "corpora":corpus_info, "sources":sources.iter().map(|s| json!({
            "corpus_id":s.id,"corpus_name":s.name,"source_fingerprint":s.fingerprint,
            "selection":s.selection
        })).collect::<Vec<_>>()});
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let manifest_hash = artifact_cache::sha256(&manifest_bytes);
    fs::create_dir_all(output.join("selections"))?;
    artifact_cache::atomic_write(
        &output
            .join("selections")
            .join(format!("{manifest_hash}.json")),
        &manifest_bytes,
    )?;
    artifact_cache::atomic_write(&output.join("selection-manifest.json"), &manifest_bytes)?;
    if let Some(path) = &args.selection_manifest {
        let path = resolve_path(&root, path.clone());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        artifact_cache::atomic_write(&path, &manifest_bytes)?;
    }
    if args.selection_only {
        return publish(
            &output,
            &records,
            &corpus_info,
            false,
            false,
            "selection ready",
        );
    }
    startup.update("initializing_run", "", 0, None)?;
    if args.aggressive_first || args.preparation_hash_profile.is_some() {
        // Set before any worker threads are created; child coordinators inherit
        // the autonomous protocol selected by this explicit experiment option.
        std::env::set_var("EXCEL_SHRINK_CAPTURE_SWEEP", "1");
        std::env::set_var(
            "EXCEL_SHRINK_EXCEL_SLOT_ROOT",
            output.join("control-room"),
        );
        std::env::set_var(
            "EXCEL_SHRINK_CAPTURE_GROUP_LIMIT",
            args.comparison_workers.to_string(),
        );
    }
    let mut run = provenance::start_run(
        &output,
        args.machine_label.as_deref(),
        manifest,
        json!({
            "prepare_only":args.prepare_only,"compare_only":args.compare_only,
            "full_scan_deferred":args.full_scan_deferred,
            "resident_capture":args.resident_capture,"comparison_workers":args.comparison_workers,
            "targeted_comparison":args.targeted_comparison,
            "targeted_skip_check_performance":args.targeted_skip_check_performance,
            "chart_capture_mode":args.chart_capture_mode,
            "retain_equal_images":args.retain_equal_images,
            "hide_excel":args.hide_excel,
            "control_room":args.control_room,"control_room_port":args.control_room_port,
            "adaptive_concurrency":args.adaptive_concurrency,"order":args.order,
            "largest_reduction_first":args.largest_reduction_first,
            "comparison_scheduling":if args.control_room { "manual-groups" } else if args.adaptive_concurrency { "adaptive-groups" } else { "fixed-batches" },
            "diagnose_whitespace_only":args.diagnose_whitespace_only,"retry_skipped":args.should_retry_skipped(),
            "workbook_timeout_seconds":args.workbook_timeout_seconds,
            "startup_timeout_seconds":args.startup_timeout_seconds,"max_workbook_bytes":args.max_workbook_bytes,
            "limit":args.limit,"reuse_prepared_from":args.reuse_prepared_from,"aggressive_first":args.aggressive_first,
            "preparation_hash_profile":args.preparation_profile(),
            "worker_memory_estimate_mib":std::env::var("EXCEL_SHRINK_WORKER_MEMORY_MIB").unwrap_or_else(|_|"4096".into()),
            "excel_memory_estimate_mib":std::env::var("EXCEL_SHRINK_EXCEL_MEMORY_MIB").unwrap_or_else(|_|"4096".into()),
            "resident_memory_estimate_mib":std::env::var("EXCEL_SHRINK_RESIDENT_MEMORY_MIB").unwrap_or_else(|_|"16384".into())
        }),
    )?;
    if let Some(room) = &room {
        room.set_run(&run, "preparing")?;
    }
    if room.is_some() {
        progress::initialize(&output, &sources, &records, &run)?;
    }
    startup.finish()?;
    let limits = ArchiveArgs {
        urls: Vec::new(),
        request_file: None,
        download_root: downloads,
        output_root: Some(output.clone()),
        workbook_timeout_seconds: args.workbook_timeout_seconds,
        startup_timeout_seconds: args.startup_timeout_seconds,
        max_success_per_archive: None,
        retry_skipped: args.should_retry_skipped(),
        max_workbook_bytes: args.max_workbook_bytes,
    };
    let mut limited = false;
    let mut missing_prepared = false;
    let producer = artifact_cache::hash_file(&std::env::current_exe()?)?;
    // Freeze each source's pending selection before preparation changes records.
    // The comparison pass must use the same workbooks even if preparation skips one.
    let selected_by_source: Vec<Vec<(usize, String)>> = sources
        .iter()
        .map(|source| {
            if args.diagnose_whitespace_only {
                return Vec::new();
            }
            let mut pending = if args.targeted_comparison {
                selection::ordered_members(source, args.order).into_iter().filter(|(index, name)| {
                    let id = identity(source, *index, name);
                    records.iter().find(|record| record["identity"] == id).is_none_or(|record| {
                        prepared::targeted::needs_attempt(
                            record,
                            args.targeted_skip_check_performance,
                            args.chart_capture_mode,
                            args.should_retry_skipped(),
                        )
                    })
                }).collect()
            } else if let Some(profile) = args.preparation_hash_profile.as_deref() {
                selection::ordered_members(source, args.order).into_iter().filter(|(index, name)| {
                    let id = identity(source, *index, name);
                    records.iter().find(|record| record["identity"] == id).is_none_or(|record| {
                        if record["status"] == "skipped" {
                            return args.should_retry_skipped();
                        }
                        record["complete"] != true
                            || !has_preparation_for_screen(record, args.preparation_hash_profile.as_deref())
                            || (record["preparation_disposition"] == "needs_visual"
                                && (record["last_hash_screen_profile"] != profile
                                    || record["last_hash_capture_policy"] != crate::regression::office_capture::ResidentCapture::CAPTURE_POLICY))
                    })
                }).collect()
            } else if args.aggressive_first {
                // Historical tall-sheet or targeted attempts may be terminal in
                // the ordinary workflow without full coverage. Include them.
                selection::ordered_members(source, args.order).into_iter().filter(|(index, name)| {
                    let id = identity(source, *index, name);
                    records.iter().find(|record| record["identity"] == id).is_none_or(|record| {
                        !strategy::full_complete(record)
                            && (record["status"] != "skipped" || args.should_retry_skipped())
                    })
                }).collect()
            } else if args.full_scan_deferred {
                selection::ordered_members(source, args.order)
                    .into_iter()
                    .filter(|(index, name)| {
                        let id = identity(source, *index, name);
                        records.iter().find(|record| record["identity"] == id)
                            .is_some_and(has_deferred_full_scan)
                    })
                    .collect()
            } else {
                selection::pending_members(
                    source,
                    &records,
                    args.order,
                    args.should_retry_skipped(),
                )
            };
            if !args.targeted_comparison && args.preparation_hash_profile.is_none()
                && !args.aggressive_first
                && !args.full_scan_deferred {
                let queued: HashSet<_> = pending.iter().map(|(index, _)| *index).collect();
                pending.extend(selection::ordered_members(source, args.order)
                    .into_iter()
                    .filter(|(index, name)| {
                        if queued.contains(index) {
                            return false;
                        }
                        let id = identity(source, *index, name);
                        records.iter().find(|record| record["identity"] == id)
                            .is_some_and(|record| {
                                record["last_hash_screen_profile"].is_string()
                                    && !strategy::full_complete(record)
                            })
                    }));
            }
            // A completed result from an older build can be in the selected
            // partition but lack the shrink evidence needed by this workflow.
            // Prepare it again instead of silently treating it as eligible.
            if !args.full_scan_deferred && !args.compare_only {
                let queued: HashSet<_> = pending.iter().map(|(index, _)| *index).collect();
                pending.extend(selection::ordered_members(source, args.order)
                    .into_iter()
                    .filter(|(index, name)| {
                        if queued.contains(index) {
                            return false;
                        }
                        let id = identity(source, *index, name);
                        records.iter().find(|record| record["identity"] == id)
                            .is_some_and(|record| {
                                matches!(record["status"].as_str(),
                                    Some("unchanged" | "differences" | "paused-for-later"
                                        | "crc-identical" | "workbook-only"))
                                    && !has_preparation_for_screen(record, args.preparation_profile())
                            })
                    }));
            }
            if args.aggressive_first && args.order == selection::ProcessingOrder::Priority {
                pending.sort_by_key(|(index, name)| member_priority(&records, &identity(source, *index, name)));
            }
            if let Some(list) = &workbook_list {
                limited |= selection::select_workbooks(source, &mut pending, list);
            }
            let count = args
                .limit
                .map(|n| n as usize)
                .unwrap_or(pending.len())
                .min(pending.len());
            limited |= count < pending.len();
            pending.into_iter().take(count).collect()
        })
        .collect();
    let current_work: HashSet<String> = sources.iter().zip(&selected_by_source)
        .flat_map(|(source, selected)| selected.iter()
            .map(|(index, name)| identity(source, *index, name)))
        .collect();
    let completed_ids: HashSet<&str> = records.iter()
        .filter(|record| record["complete"] == true)
        .filter_map(|record| record["identity"].as_str())
        .collect();
    let selected_ids: HashSet<String> = sources.iter()
        .flat_map(|source| source.members.iter()
            .filter(|(index, _)| source.selection.contains(*index))
            .map(|(index, name)| identity(source, *index, name)))
        .filter(|id| !limited || current_work.contains(id) || completed_ids.contains(id.as_str()))
        .collect();
    progress::set_selection(&output, &selected_ids);
    if args.preparation_hash_profile.is_some() {
        progress::set_comparison_pass(&output, strategy::HASH_SCREEN);
    }
    if (!args.compare_only || args.preparation_hash_profile.is_some()) && !args.diagnose_whitespace_only {
        preparation::run_all(
            &sources,
            &selected_by_source,
            &args,
            &manifest_hash,
            &producer,
            &output,
            &mut records,
            &corpus_info,
            &limits,
            &run,
            log_level,
            limited,
        )?;
    }
    let mut visual_scope_frozen = false;
    // Screening spans the complete selection before any full secondary pass.
    let comparison_passes: &[&str] = if args.preparation_hash_profile.is_some() {
        // The sweep itself persists the verdict and mismatch images. Never
        // reopen failed or changed sheets through the legacy visual pipeline.
        progress::set_comparison_pass(&output, strategy::HASH_SCREEN);
        progress::freeze_visual_scope(&output);
        &[]
    } else if args.aggressive_first {
        &[strategy::PRIMARY, strategy::SECONDARY]
    } else {
        &["full"]
    };
    for &comparison_pass in comparison_passes {
        run["options"]["comparison_pass"] = json!(comparison_pass);
        progress::set_comparison_pass(&output, comparison_pass);
        if let Some(room) = &room {
            room.set_run(&run, if comparison_pass == strategy::PRIMARY { "screening" } else { "comparing" })?;
        }
    // Comparison starts only after every selected preparation has finished.
    for (source, selected) in sources.iter().zip(&selected_by_source) {
        if !visual_scope_frozen && !args.diagnose_whitespace_only {
            // Freeze eligibility only after every selected workbook has reached
            // a terminal preparation result, including reused preparations.
            let mut incomplete = Vec::new();
            for (prepared_source, prepared_members) in sources.iter().zip(&selected_by_source) {
                for (index, name) in prepared_members {
                    ensure_record(&mut records, prepared_source, *index, name);
                }
            }
            let mut observed = HashSet::new();
            for record in &mut records {
                let Some(id) = record["identity"].as_str().map(str::to_owned) else {
                    continue;
                };
                if !selected_ids.contains(&id) {
                    continue;
                }
                observed.insert(id.clone());
                if classify_preparation(record) {
                    write_record(&record_path(&output, &id), record)?;
                }
                if !record["preparation_disposition"].is_string() {
                    let name = record["name"].as_str().unwrap_or("unknown workbook");
                        if args.compare_only {
                            record["preparation_disposition"] = json!("evidence_missing");
                            record["status"] = json!("skipped");
                            record["complete"] = json!(true);
                            record["error"] = json!("No prepared bundle with required shrink evidence; run preparation with this build.");
                            write_record(&record_path(&output, &id), record)?;
                            missing_prepared = true;
                        } else {
                            incomplete.push(name.to_owned());
                        }
                }
                progress::observe(&output, record);
            }
            incomplete.extend(selected_ids.difference(&observed).cloned());
            if !incomplete.is_empty() {
                publish_progress(&output, &records, &corpus_info, "preparation incomplete");
                anyhow::bail!("Preparation ended before a terminal result for {} selected workbooks; first: {}",
                    incomplete.len(), incomplete[0]);
            }
            progress::freeze_visual_scope(&output);
            publish_progress(&output, &records, &corpus_info, "prepared");
            visual_scope_frozen = true;
        }
        // Freeze membership over the entire corpus before removing completed files.
        let members = selection::ordered_members(source, args.order);
        // Completed differences get their missing causal diagnosis before new work.
        // The primary measurements and comparisons are never reset for this pass.
        if !args.targeted_comparison && !args.prepare_only && !args.compare_only && !args.aggressive_first
            && args.preparation_hash_profile.is_none() {
            let backfill: Vec<_> = members
                .iter()
                .filter(|(index, name)| {
                    let id = identity(source, *index, name);
                    records
                        .iter()
                        .find(|r| r["identity"] == id)
                        .is_some_and(whitespace::needed)
                })
                .cloned()
                .collect();
            limited |= args
                .limit
                .is_some_and(|limit| limit < backfill.len() as u64);
            for (index, name) in backfill
                .iter()
                .take(args.limit.unwrap_or(u64::MAX) as usize)
            {
                let id = identity(source, *index, name);
                let pos = records.iter().position(|r| r["identity"] == id).unwrap();
                records[pos]["run_selection"] = source.selection.provenance(
                    *index,
                    &source.id,
                    &source.fingerprint,
                    &manifest_hash,
                    args.order,
                );
                process(
                    source,
                    *index,
                    &id,
                    pos,
                    JobMode::Diagnose,
                    &output,
                    &mut records,
                    &corpus_info,
                    &limits,
                    &run,
                    log_level,
                )?;
            }
        }
        if args.diagnose_whitespace_only {
            continue;
        }
        // Control-room admission already archives its previous attempts. Keep
        // the same history for CLI retries before preparation clears old errors.
        if !args.control_room && args.compare_only {
            for (index, name) in selected {
                let id = identity(source, *index, name);
                if let Some(record) = records.iter().find(|record| {
                    record["identity"] == id
                        && record["complete"] == true
                        && record["status"] == "skipped"
                }) {
                    control_room::archive_record(&output.join("control-room"), &id, record)?;
                }
            }
        }
        if !args.prepare_only {
            let mut comparisons = Vec::new();
            for (index, name) in selected {
                let id = identity(source, *index, name);
                let pos = ensure_record(&mut records, source, *index, name);
                if let Some(profile) = &args.preparation_hash_profile {
                    records[pos]["comparison_hash_profile"] = json!(profile);
                }
                if (args.aggressive_first || args.preparation_hash_profile.is_some())
                    && !strategy::needs_pass(&records[pos], comparison_pass) {
                    continue;
                }
                if records[pos]["preparation_disposition"] != "needs_visual" {
                    if args.compare_only
                        && (records[pos]["preparation_disposition"].is_null()
                            || records[pos]["preparation_disposition"] == "evidence_missing")
                    {
                        eprintln!("[study] Not prepared with required evidence: {name}");
                        missing_prepared = true;
                    }
                    continue;
                }
                if !reusable_preparation(
                    &records[pos],
                    &bundle_path(&output, &id),
                    &producer,
                    &source.fingerprint,
                    &args.reuse_prepared_from,
                    args.preparation_profile(),
                ) {
                    if args.compare_only && records[pos]["status"] != "skipped" {
                        eprintln!("[study] Not prepared with this build: {name}");
                        missing_prepared = true;
                    }
                    continue;
                }
                records[pos]["compared_by"] = json!(producer);
                if args.full_scan_deferred {
                    records[pos]["full_scan_deferred"] = json!(true);
                }
                records[pos]["run_selection"] = source.selection.provenance(
                    *index,
                    &source.id,
                    &source.fingerprint,
                    &manifest_hash,
                    args.order,
                );
                comparisons.push(scheduler::Comparison {
                    index: *index,
                    id,
                    position: pos,
                });
            }
            if args.largest_reduction_first {
                reduction_order::sort(&mut comparisons, &records);
            }
            if args.targeted_comparison {
                progress::queue_targeted_comparisons(
                    &output,
                    &comparisons.iter().map(|job| job.id.clone()).collect::<Vec<_>>(),
                );
            }
            scheduler::compare(
                source,
                comparisons,
                args.comparison_workers,
                &output,
                &mut records,
                &corpus_info,
                &limits,
                &run,
                log_level,
            )?;
        }
    }
    }
    if args.control_room
        && !args.targeted_comparison
        && args.resident_capture
        && !args.aggressive_first
        && args.preparation_hash_profile.is_none()
        && !args.prepare_only
        && !args.diagnose_whitespace_only
        && !args.full_scan_deferred
    {
        scheduler::await_deferred_manual(
            &sources,
            &output,
            &mut records,
            &corpus_info,
            &limits,
            &run,
            log_level,
        )?;
    }
    if args.control_room {
        control_room::check_stop(&output.join("control-room"))?;
    }
    let finished = !args.prepare_only && !limited && selection::complete(&corpus_info, &records);
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
    export_results(&args, &root, &output, &lock)?;
    if let Some(room) = room.as_mut() {
        progress::finish(&output);
        room.finish()?;
    }
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

fn export_results(args: &StudyArgs, root: &Path, output: &Path, lock: &fs::File) -> Result<()> {
    if let Some(package) = &args.export_results {
        let receipt = transfer::export_package_locked(output, &resolve_path(root, package.clone()), lock)?;
        eprintln!("[study] Result export: {receipt}");
    }
    Ok(())
}

fn corpus_id(corpus: &Corpus) -> Result<String> {
    match &corpus.id {
        Some(id) => preparation_build_hash(id)
            .map_err(anyhow::Error::msg)
            .context("corpus id must be a full SHA-256"),
        None => Ok(artifact_cache::sha256(corpus.url.as_bytes())),
    }
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

// Serial preparation and fallback share one cadence across workbook boundaries.
// Call only after saving the result and releasing its active queue entry, never
// from the worker watchdog's progress callback.
fn report_refresh_due(output: &Path) -> bool {
    fs::metadata(output.join("diagnosis.json"))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_none_or(|elapsed| elapsed >= Duration::from_secs(30))
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
    let records: Vec<_> = records
        .iter()
        .cloned()
        .map(|mut record| {
            diagnosis_report::ensure_whitespace_diagnosis(&mut record);
            record
        })
        .collect();
    let limited = limited || selection::restricted(corpora);
    let mut report = json!({"archive_run":true,"study":true,"workbooks":records,"corpora":corpora,"total_workbooks":total,"complete":complete && !limited,"limited":limited,"phase":phase});
    report["measurement_provenance"] = provenance::report_metadata(output, &records)?;
    artifact_cache::atomic_write(
        &output.join("diagnosis.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    if phase != "loading archives" && phase != "selection ready" {
        write_workbook_index(output, &records)?;
    }
    ledger::write(output, &records)?;
    diagnosis_report::write(output, &report).map(|_| ())
}

fn has_marked_sheets(value: &Value) -> bool {
    match value {
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => fields.values().any(has_marked_sheets),
        _ => false,
    }
}

fn fully_visually_identical(record: &Value) -> bool {
    if prepared::available_profiles(record).len() != prepared::PROFILES.len()
        || record["status"] != "unchanged"
        || record["preparation_disposition"] != "needs_visual"
        || !has_required_shrink_evidence(record)
        || has_marked_sheets(&record["capture_deferred"])
    {
        return false;
    }
    let Some(profiles) = record["results"].as_array() else {
        return false;
    };
    let mut visually_checked = 0;
    let all_complete = profiles.iter().all(|profile| {
        let shrink = &profile["shrink_identity"];
        if shrink["workbook_original_archive_reused"] == true
            || shrink["workbook_only_cleanup_available"] == true
        {
            return true;
        }
        visually_checked += 1;
        profile["checks_complete"] == true
            && profile["status"] == "visually unchanged"
            && profile["failures"].as_array().is_none_or(|errors| errors.is_empty())
    });
    all_complete && visually_checked > 0
}

fn write_workbook_index(output: &Path, records: &[Value]) -> Result<()> {
    let Some((run_id, selected, visual_scope_frozen)) = progress::selection_scope(output) else {
        return Ok(());
    };
    let workbooks: Vec<_> = records.iter().filter_map(|record| {
        let identity = record["identity"].as_str()?;
        if !selected.contains(identity) {
            return None;
        }
        let available = record["prepared"] == true && bundle_path(output, identity).is_file();
        let visually_identical = fully_visually_identical(record);
        let has_reused_comparison = record["results"].as_array().is_some_and(|profiles| profiles.iter()
            .any(|profile| profile["comparison_reused_from"].is_string()));
        Some(json!({
            "id":control_room::group_id(identity),
            "name":record["name"],"corpus":record["corpus_name"],
            "preparation_disposition":record["preparation_disposition"],
            "reason_code":record["namespace_exclusion"]["code"].as_str()
                .or_else(|| record["preparation_disposition"].as_str()),
            "status":record["status"],"error":record["error"],
            "crc_equal_after_shrink":record["preparation_disposition"] == "crc_unchanged",
            "visually_identical":visually_identical,
            "checked_no_differences":visually_identical || (record["complete"] == true
                && record["status"] == "unchanged"
                && record["targeted_comparison"]["status"] == "completed_with_limitations"),
            "differences_found":matches!(record["status"].as_str(), Some("differences" | "screening-differences")),
            "priority_validation":record["priority_validation"],
            "diagnosis_categories":strategy::diagnosis_categories(record),
            "default_regression_candidate":strategy::default_regression_candidate(record),
            "has_capture_deferred":has_marked_sheets(&record["capture_deferred"]),
            "has_recalc_identity_changed":has_marked_sheets(&record["recalc_identity_changed"]),
            "has_reused_comparison":has_reused_comparison,
            "baseline_repaired":record["baseline_repaired"],
            "source_format":record["source_format"],
            "profile_equivalence":record["profile_equivalence"],
            "available_variants":{"original":available,"default":available,
                "maximum-cleaning":available && record["results"].as_array().is_some_and(|rows|
                    rows.iter().any(|row| row["profile"] == "maximum-cleaning")),
                "too-aggressive-cleaning":available && record["results"].as_array().is_some_and(|rows|
                    rows.iter().any(|row| row["profile"] == "too-aggressive-cleaning"))}
        }))
    }).collect();
    let index = json!({"version":1,"run_id":run_id,
        "visual_scope_frozen":visual_scope_frozen,"workbooks":workbooks});
    artifact_cache::atomic_write(&output.join("control-room/workbook-index.json"),
        &serde_json::to_vec(&index)?)
}

pub(super) fn run_job(job: &MemberJob, record: &mut Value, log_level: LogLevel) -> Result<()> {
    let outcome = match job.mode {
        JobMode::Prepare => prepared::prepare(job, record, log_level),
        JobMode::Compare => prepared::compare(job, record),
        JobMode::Diagnose => {
            whitespace::run(job, record, log_level);
            Ok(())
        }
        JobMode::Legacy => unreachable!(),
    };
    if let Some(run) = &job.run_manifest {
        let stage = match job.mode {
            JobMode::Prepare => provenance::Stage::Preparation,
            JobMode::Compare => provenance::Stage::Comparison,
            JobMode::Diagnose => provenance::Stage::WhitespaceDiagnosis,
            JobMode::Legacy => unreachable!(),
        };
        provenance::stamp(record, run, stage);
        if job.mode == JobMode::Compare && record["whitespace_diagnosis"]["version"].is_number() {
            provenance::stamp(record, run, provenance::Stage::WhitespaceDiagnosis);
        }
    }
    outcome
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

/// Accept a bundle only for a reviewed producer, matching source and requested profiles.
fn reusable_preparation(
    record: &Value,
    bundle: &Path,
    producer: &str,
    source: &str,
    compatible_builds: &[String],
    screen_profile: Option<&str>,
) -> bool {
    record["prepared"] == true
        && bundle.is_file()
        && (record["prepared_by"] == producer
            || record["preparation_accepted_by"] == producer
            || compatible_builds
                .iter()
                .any(|hash| record["prepared_by"] == *hash))
        && record["source_fingerprint"] == source
        && has_preparation_for_screen(record, screen_profile)
}

/// Validate the stored evidence and every profile needed by this request.
/// A partial bundle can satisfy a narrow screen without satisfying a later
/// wider request; comparison completion alone must not skip that preparation.
fn has_preparation_for_screen(record: &Value, screen_profile: Option<&str>) -> bool {
    has_required_shrink_evidence(record)
        && prepared::preparation_profiles(screen_profile)
            .iter()
            .all(|name| {
                let config = &predefined_visual_profiles()
                    .iter()
                    .find(|p| p.name == *name)
                    .unwrap()
                    .config;
                record["results"].as_array().is_some_and(|profiles| {
                    profiles.iter().filter(|p| p["profile"] == *name).count() == 1
                        && profiles.iter().any(|p| {
                            p["profile"] == *name && p["config_hash"] == config.config_hash()
                        })
                })
            })
}

fn has_required_shrink_evidence(record: &Value) -> bool {
    let Some(profiles) = record["results"].as_array() else {
        return false;
    };
    let names = prepared::available_profiles(record);
    !profiles.is_empty()
        && names.len() == profiles.len()
        && names.iter().all(|name| {
            let Some(profile) = profiles.iter().find(|profile| profile["profile"] == *name) else {
                return false;
            };
            let evidence = &profile["shrink_identity"];
            evidence["workbook_original_archive_reused"].is_boolean()
                && evidence["workbook_only_cleanup_available"].is_boolean()
                && evidence["worksheet_xml_changed"].is_boolean()
                && evidence["cleaned_package_crc_length_sha256"]
                    .as_str()
                    .is_some_and(|hash| hash.len() == 64)
                && evidence["workbook_xml_cleaned_entries"].is_array()
                && evidence["sheets"].is_object()
                && profile["cleanup_analysis"].is_object()
        })
}

fn invalidate_preparation_claim(record: &mut Value) {
    let fields = record.as_object_mut().expect("workbook record is an object");
    for name in [
        "preparation_accepted_by",
        "preparation_disposition",
        "namespace_exclusion",
        "profile_equivalence",
        "package_identity",
    ] {
        fields.remove(name);
    }
    record["prepared"] = json!(false);
    record["complete"] = json!(false);
    record["results"] = json!([]);
    record["shrink_identity"] = json!({});
    record["cleanup_analysis"] = json!({});
    record["baseline_repaired"] = Value::Null;
}

fn classify_preparation(record: &mut Value) -> bool {
    if record["namespace_exclusion"].is_object() {
        if record["preparation_disposition"] == "unsupported_namespace" {
            return false;
        }
        record["preparation_disposition"] = json!("unsupported_namespace");
        record["complete"] = json!(true);
        return true;
    }

    if record["prepared"] == true {
        if !has_required_shrink_evidence(record) {
            if record["preparation_disposition"] == "evidence_missing" {
                return false;
            }
            record["preparation_disposition"] = json!("evidence_missing");
            record["status"] = json!("skipped");
            record["error"] = json!("Prepared workbook lacks required shrink evidence; prepare it again with this build.");
            record["complete"] = json!(true);
            return true;
        }
        let identities: Vec<_> = record["results"].as_array().unwrap().iter()
            .filter_map(|profile| profile["profile"].as_str()
                .map(|name| (name.to_owned(), profile["shrink_identity"].clone())))
            .collect();
        record["shrink_identity"] = json!({});
        for (name, identity) in identities {
            record["shrink_identity"][name] = identity;
        }
        let nonvisual = prepared::finalize_nonvisual_preparation(record);
        let disposition = if nonvisual {
            if record["status"] == "workbook-only" { "workbook_only" } else { "crc_unchanged" }
        } else {
            "needs_visual"
        };
        if !nonvisual
            && record["complete"] == true
            && matches!(record["status"].as_str(),
                Some("unchanged" | "differences" | "screened-equal" | "paused-for-later" | "skipped"))
        {
            if record["preparation_disposition"] == "needs_visual" {
                return false;
            }
            record["preparation_disposition"] = json!("needs_visual");
            return true;
        }
        let already_classified = record["preparation_disposition"] == disposition
            && record["complete"] == nonvisual
            && (nonvisual || record["status"] == "prepared");
        if already_classified {
            return false;
        }
        record["preparation_disposition"] = json!(disposition);
        record["complete"] = json!(nonvisual);
        if !nonvisual {
            record["status"] = json!("prepared");
            record["error"] = Value::Null;
        }
        return true;
    }

    if record["status"] == "skipped" {
        if record["preparation_disposition"] == "preparation_failed" {
            return false;
        }
        record["preparation_disposition"] = json!("preparation_failed");
        record["complete"] = json!(true);
        return true;
    }
    false
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
