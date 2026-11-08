//! Prepare independent, attributable optimization artifacts with continuously reused Excel.
use super::*;
use crate::validation_preparation::{self, Input};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
mod reuse;

#[derive(Default)]
struct ProgressCounters {
    prepared: AtomicUsize,
    failed: AtomicUsize,
    reused: AtomicUsize,
}

/// Publish compact counters without rescanning per-workbook receipts for web status queries.
fn publish_progress(
    output: &Path,
    counters: &ProgressCounters,
    started: usize,
    total: usize,
    workers: usize,
    elapsed: Duration,
    status: &str,
) -> Result<()> {
    let prepared = counters.prepared.load(Ordering::Acquire);
    let failed = counters.failed.load(Ordering::Acquire);
    let completed = prepared + failed;
    let started = started.min(total).max(completed);
    write_json(
        &output.join("progress.json"),
        &json!({
            "version":1,"operation":"check-performance","status":status,
            "total":total,"started":started,"prepared":prepared,"failed":failed,
            "reused":counters.reused.load(Ordering::Acquire),
            "in_flight":started.saturating_sub(completed),"queued":total.saturating_sub(started),
            "unresolved":total.saturating_sub(completed),"requested_workers":workers,
            "elapsed_seconds":elapsed.as_secs_f64(),"updated_at":chrono::Utc::now().to_rfc3339()
        }),
    )
}

#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Prepared XLSX inputs. Legacy conversion remains a separate operation.
    #[arg(long, required_unless_present = "source_manifest", conflicts_with = "source_manifest", num_args = 1..)]
    input: Vec<PathBuf>,
    /// Stable source occurrences and expected baseline hashes for corpus reuse.
    #[arg(long)]
    source_manifest: Option<PathBuf>,
    /// New directory retaining per-input results, failures and worker ownership.
    #[arg(long)]
    output: PathBuf,
    /// Independent Excel workers; may exceed the logical processor count.
    #[arg(long, default_value_t = 2)]
    workers: usize,
    /// Reuse verified successful inputs from a completed or positively departed preparation.
    #[arg(long)]
    reuse_from: Option<PathBuf>,
}

/// Run one continuously refilled queue, retaining every input's terminal receipt.
pub(crate) fn run_manifest(
    source_manifest: PathBuf,
    output: PathBuf,
    workers: usize,
    reuse_from: Option<PathBuf>,
) -> Result<()> {
    run(Args {
        input: Vec::new(),
        source_manifest: Some(source_manifest),
        output,
        workers,
        reuse_from,
    })
}

/// Run one continuously refilled queue, retaining every input's terminal receipt.
pub(crate) fn run(args: Args) -> Result<()> {
    anyhow::ensure!(
        args.workers > 0,
        "Check Performance workers must be positive"
    );
    anyhow::ensure!(
        !args.output.exists(),
        "Choose a new preparation output directory"
    );
    let inputs = match &args.source_manifest {
        Some(path) => validation_preparation::read(path)?,
        None => validation_preparation::explicit(&args.input)?,
    };
    let selection_hash = validation_preparation::selection_hash(&inputs)?;
    let previous = args
        .reuse_from
        .as_deref()
        .map(reuse::Index::open)
        .transpose()?;
    fs::create_dir(&args.output)?;
    let output = fs::canonicalize(&args.output)?;
    write_json(
        &output.join("control.json"),
        &json!({"version":1,"workers":args.workers}),
    )?;
    let producer = super::super::artifact_cache::hash_file(&std::env::current_exe()?)?;
    write_json(
        &output.join("owner.json"),
        &json!({"pid":std::process::id(),"started_ms":crate::native_process::current_started_unix_ms()}),
    )?;
    let mut pool = Pool::new_performance(output.join("workers"))?;
    let pool_root = pool.root.clone();
    let next = AtomicUsize::new(0);
    let counters = ProgressCounters::default();
    let started_at = Instant::now();
    let recipe = generation_recipe(&pool_root)?;
    let mut retained = Vec::new();
    let mut pending = Vec::new();
    let mut reuse_lookups = std::collections::BTreeMap::new();
    let mut last_restore_progress = Instant::now();
    publish_progress(
        &output,
        &counters,
        0,
        inputs.len(),
        args.workers,
        started_at.elapsed(),
        "restoring",
    )?;
    for (index, input) in inputs.iter().enumerate() {
        let directory = output.join(format!("input-{index:06}"));
        let lookup = previous
            .as_ref()
            .map(|previous| previous.lookup(input, &recipe));
        match lookup {
            Some(Ok(Some(entry))) => {
                entry.restore(&directory)?;
                retained.push(json!({"index":index,"source_id":input.source_id,"name":input.name,
                    "input":input.baseline,"directory":directory,"status":"prepared","error":null,"reused":true}));
                counters.prepared.fetch_add(1, Ordering::Release);
                counters.reused.fetch_add(1, Ordering::Release);
            }
            other => {
                let detail = match other {
                    Some(Err(error)) => {
                        format!("incompatible or invalid prior evidence: {error:#}")
                    }
                    _ => "no compatible completed input".into(),
                };
                if previous.is_some() {
                    reuse_lookups.insert(index, detail);
                }
                pending.push(index);
            }
        }
        if last_restore_progress.elapsed() >= Duration::from_secs(1) {
            publish_progress(
                &output,
                &counters,
                retained.len(),
                inputs.len(),
                args.workers,
                started_at.elapsed(),
                "restoring",
            )?;
            last_restore_progress = Instant::now();
        }
    }
    let reused = retained.len();
    let outcomes = Mutex::new(retained);
    let count = args.workers.min(pending.len());
    let target = AtomicUsize::new(count);
    let requested_target = AtomicUsize::new(args.workers);
    let peak_dispatch_slots = AtomicUsize::new(count);
    publish_progress(
        &output,
        &counters,
        reused,
        inputs.len(),
        args.workers,
        started_at.elapsed(),
        "running",
    )?;
    let pool_result = std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        let mut last_control = Instant::now();
        loop {
            let desired = target.load(Ordering::Acquire);
            for slot in handles.len()..desired {
                let target = &target;
                let next = &next;
                let pending = &pending;
                let inputs = &inputs;
                let output = &output;
                let pool_root = &pool_root;
                let producer = &producer;
                let outcomes = &outcomes;
                let counters = &counters;
                let reuse_lookups = &reuse_lookups;
                handles.push(scope.spawn(move || loop {
                    if next.load(Ordering::Acquire) >= pending.len() {
                        break;
                    }
                    if slot >= target.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&index) = pending.get(position) else {
                        break;
                    };
                    let input = &inputs[index];
                    let directory = output.join(format!("input-{index:06}"));
                    let result = prepare(&pool_root, input, &directory, &producer);
                    let succeeded = result.is_ok();
                    let outcome = json!({
                        "index": index,
                        "source_id": input.source_id,
                        "name": input.name,
                        "input": input.baseline,
                        "directory": directory,
                        "status": if result.is_ok() { "prepared" } else { "failed" },
                        "error": result.err().map(|error| format!("{error:#}")),
                        "reused":false,"reuse_lookup":reuse_lookups.get(&index)
                    });
                    if outcome["status"] == "failed" {
                        if let Err(error) = write_json(&directory.join("failed.json"), &outcome) {
                            eprintln!("Cannot retain preparation failure: {error:#}");
                        }
                    }
                    outcomes
                        .lock()
                        .expect("preparation outcomes lock poisoned")
                        .push(outcome);
                    if succeeded {
                        counters.prepared.fetch_add(1, Ordering::Release);
                    } else {
                        counters.failed.fetch_add(1, Ordering::Release);
                    }
                }));
            }
            peak_dispatch_slots.store(handles.len(), Ordering::Release);
            if handles.iter().all(|handle| handle.is_finished()) {
                break;
            }
            if let Err(error) = pool.reconcile(desired, 512) {
                pool.abort(&error)?;
                return Err(error);
            }
            if last_control.elapsed() >= Duration::from_secs(1) {
                let requested_workers = match read_worker_target(&output.join("control.json")) {
                    Ok(workers) => {
                        requested_target.store(workers, Ordering::Release);
                        target.store(workers.min(pending.len()), Ordering::Release);
                        workers
                    }
                    Err(error) => {
                        pool.abort(&error)?;
                        return Err(error);
                    }
                };
                if let Err(error) = publish_progress(
                    &output,
                    &counters,
                    reused + next.load(Ordering::Acquire).min(pending.len()),
                    inputs.len(),
                    requested_workers,
                    started_at.elapsed(),
                    "running",
                ) {
                    pool.abort(&error)?;
                    return Err(error);
                }
                last_control = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Check Performance worker thread panicked"))?;
        }
        Ok(())
    });
    drop(pool);
    let mut outcomes = outcomes
        .into_inner()
        .map_err(|_| anyhow::anyhow!("Preparation outcomes poisoned"))?;
    outcomes.sort_by_key(|row| row["index"].as_u64());
    let failed = outcomes
        .iter()
        .filter(|row| row["status"] == "failed")
        .count();
    write_json(
        &output.join("preparation.json"),
        &json!({
            "version": 1,
            "operation": "check-performance",
            "workers": peak_dispatch_slots.load(Ordering::Acquire),
            "initial_dispatch_slots": count,
            "requested_workers": args.workers,
            "final_requested_workers": requested_target.load(Ordering::Acquire),
            "producer_sha256": producer,
            "selection_sha256": selection_hash,
            "identity_scope": if args.source_manifest.is_some() { "source-manifest" } else { "explicit-file-list" },
            "manager_error": pool_result.as_ref().err().map(|error| format!("{error:#}")),
            "visual_validation": "pending",
            "total": inputs.len(),
            "failed": failed,
            "reused":reused,
            "reuse_from":args.reuse_from,
            "inputs": outcomes
        }),
    )?;
    publish_progress(
        &output,
        &counters,
        reused + next.load(Ordering::Acquire).min(pending.len()),
        inputs.len(),
        requested_target.load(Ordering::Acquire),
        started_at.elapsed(),
        if pool_result.is_ok() && failed == 0 {
            "completed"
        } else {
            "failed"
        },
    )?;
    pool_result?;
    anyhow::ensure!(
        failed == 0,
        "{failed} Check Performance preparations failed; inspect preparation.json"
    );
    Ok(())
}

/// Read the independently adjustable native pool target from its atomic control file.
fn read_worker_target(path: &Path) -> Result<usize> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(
        value["version"] == 1,
        "Unsupported preparation control version"
    );
    let workers = value["workers"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok());
    let workers = workers.context("Preparation worker target must be a positive integer")?;
    anyhow::ensure!(workers > 0, "Preparation worker target must be positive");
    Ok(workers)
}

fn prepare(root: &Path, source: &Input, directory: &Path, producer_sha256: &str) -> Result<()> {
    let input = &source.baseline;
    fs::create_dir(directory)?;
    let runtime = directory.join("runtime");
    fs::create_dir(&runtime)?;
    let source_hash = super::super::artifact_cache::hash_file(input)?;
    if let Some(expected) = &source.expected_sha256 {
        anyhow::ensure!(
            expected == &source_hash,
            "Prepared baseline does not match its expected source identity"
        );
    }
    let baseline = directory.join("baseline.xlsx");
    fs::copy(input, &baseline)?;
    anyhow::ensure!(
        source_hash == super::super::artifact_cache::hash_file(&baseline)?,
        "Baseline changed while freezing the preparation input"
    );
    zip::ZipArchive::new(File::open(&baseline)?)?
        .by_name("xl/workbook.xml")
        .context("Expected an XLSX workbook")?;
    let (session, lease) = client::lease_session(root)?;
    let request = Request {
        id: uuid::Uuid::new_v4().to_string(),
        input: PathBuf::from(super::super::power_shell_path(&baseline)?),
        output: PathBuf::from(super::super::power_shell_path(directory)?),
        directory: PathBuf::from(super::super::power_shell_path(&runtime)?),
        performance: true,
    };
    let response = client::run_request(root, &session, &request)?
        .context("Check Performance worker exited before completing this input")?;
    drop(lease);
    anyhow::ensure!(
        source_hash == super::super::artifact_cache::hash_file(input)?,
        "Input changed during preparation"
    );
    anyhow::ensure!(
        matches!(
            response["native_result"]["status"].as_str(),
            Some("optimization_confirmed" | "no_optimization_needed")
        ),
        "No confirmed native optimization outcome"
    );
    let mut artifacts = std::collections::BTreeMap::new();
    for name in validation_preparation::PREPARATION_CONTROLS {
        let path = directory.join(name);
        artifacts.insert(
            name,
            json!({
                "bytes": fs::metadata(&path)?.len(),
                "sha256": super::super::artifact_cache::hash_file(&path)?
            }),
        );
    }
    let recipe = generation_recipe(root)?;
    write_json(
        &directory.join("prepared.json"),
        &json!({
            "version": 1,
            "operation": "check-performance",
            "source": input,
            "producer_sha256": producer_sha256,
            "source_id": source.source_id,
            "name": source.name,
            "raw_source": source.raw_source,
            "source_sha256": source_hash,
            "baseline": {
                "file": "baseline.xlsx",
                "sha256": source_hash,
                "bytes": fs::metadata(&baseline)?.len()
            },
            "artifacts": artifacts,
            "native_result": response["native_result"],
            "measurements": response["metrics"],
            "environment": response["environment"],
            "generation_recipe": recipe,
            "visual_validation": "pending"
        }),
    )
}

fn generation_recipe(root: &Path) -> Result<Value> {
    let mut recipe = std::collections::BTreeMap::new();
    for name in [
        "worker.ps1",
        "WorkerPerformance.psm1",
        "CheckPerformance.psm1",
        "NativeWindow.cs",
        "ExcelProcessId.cs",
    ] {
        recipe.insert(
            name,
            super::super::artifact_cache::hash_file(&root.join(name))?,
        );
    }
    Ok(serde_json::to_value(recipe)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_continuation_needs_no_excel_worker_and_preserves_producer() -> Result<()> {
        let (root, previous, manifest, _) = reuse::fixture()?;
        let output = root.join("continued");
        run(Args {
            input: Vec::new(),
            source_manifest: Some(manifest),
            output: output.clone(),
            workers: 32,
            reuse_from: Some(previous),
        })?;
        let batch = read_json(&output.join("preparation.json")).unwrap();
        assert_eq!(batch["reused"], 1);
        assert_eq!(batch["workers"], 0);
        assert_eq!(batch["inputs"][0]["status"], "prepared");
        let receipt = read_json(&output.join("input-000000/prepared.json")).unwrap();
        assert_eq!(receipt["producer_sha256"], "a".repeat(64));
        assert_eq!(
            read_json(&output.join("workers/manifest.json")).unwrap()["workers"],
            json!([])
        );
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["reused"], 1);
        assert_eq!(progress["status"], "completed");
        Ok(())
    }

    #[test]
    fn compact_progress_counts_queued_in_flight_and_terminal_outcomes() -> Result<()> {
        let output =
            std::env::temp_dir().join(format!("performance-progress-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&output)?;
        let counters = ProgressCounters::default();
        counters.prepared.store(2, Ordering::Release);
        counters.failed.store(1, Ordering::Release);
        publish_progress(
            &output,
            &counters,
            5,
            10,
            32,
            Duration::from_secs(12),
            "running",
        )?;
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["prepared"], 2);
        assert_eq!(progress["failed"], 1);
        assert_eq!(progress["in_flight"], 2);
        assert_eq!(progress["queued"], 5);
        assert_eq!(progress["unresolved"], 7);
        assert_eq!(progress["requested_workers"], 32);
        assert_eq!(progress["elapsed_seconds"], 12.0);
        counters.prepared.store(9, Ordering::Release);
        publish_progress(
            &output,
            &counters,
            42,
            10,
            32,
            Duration::from_secs(20),
            "failed",
        )?;
        let progress = read_json(&output.join("progress.json")).unwrap();
        assert_eq!(progress["started"], 10);
        assert_eq!(progress["in_flight"], 0);
        assert_eq!(progress["queued"], 0);
        assert_eq!(progress["unresolved"], 0);
        assert_eq!(progress["status"], "failed");
        Ok(())
    }

    #[test]
    fn preparation_target_requires_versioned_positive_integer() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("preparation-target-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let path = root.join("control.json");
        for workers in [1, 4, 48] {
            write_json(&path, &json!({"version":1,"workers":workers}))?;
            assert_eq!(read_worker_target(&path)?, workers);
        }
        for value in [
            json!({"version":2,"workers":4}),
            json!({"version":1,"workers":0}),
            json!({"version":1,"workers":-1}),
            json!({"version":1,"workers":1.5}),
            json!({"version":1,"workers":"4"}),
        ] {
            write_json(&path, &value)?;
            assert!(read_worker_target(&path).is_err());
        }
        Ok(())
    }

    #[test]
    fn rejects_zero_workers_before_creating_output() {
        let output = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let error = run(Args {
            input: vec![],
            source_manifest: None,
            output: output.clone(),
            workers: 0,
            reuse_from: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("positive"));
        assert!(!output.exists());
    }

    #[test]
    fn old_conversion_requests_keep_the_conversion_operation() {
        let request: Request = serde_json::from_value(json!({
            "id":"old", "input":"input.xls", "output":"output.xlsx", "directory":"runtime"
        }))
        .unwrap();
        assert!(!request.performance);
    }

    #[test]
    fn expected_baseline_mismatch_fails_before_leasing_excel() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("preparation-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let baseline = root.join("source.xlsx");
        fs::write(&baseline, b"changed source")?;
        let pool = root.join("pool");
        fs::create_dir(&pool)?;
        write_json(
            &pool.join("error.json"),
            &json!({"error":"Excel leasing must not be reached"}),
        )?;
        let input = Input {
            source_id: "stable-source".into(),
            name: "source.xlsx".into(),
            baseline,
            expected_sha256: Some("a".repeat(64)),
            raw_source: None,
        };
        let error = prepare(&pool, &input, &root.join("result"), &"a".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("expected source identity"));
        assert!(!root.join("result/prepared.json").exists());
        Ok(())
    }

    #[test]
    #[ignore = "requires desktop Excel with Check Performance and PERFORMANCE_PREPARATION_FIXTURE"]
    fn native_performance_worker_reuses_excel_and_retains_controls() -> Result<()> {
        let input = PathBuf::from(std::env::var("PERFORMANCE_PREPARATION_FIXTURE")?);
        let output =
            std::env::temp_dir().join(format!("performance-pool-{}", uuid::Uuid::new_v4()));
        run(Args {
            input: vec![input.clone(), input],
            source_manifest: None,
            output: output.clone(),
            workers: 1,
            reuse_from: None,
        })?;
        let progress =
            read_json(&output.join("progress.json")).context("Progress receipt missing")?;
        assert_eq!(progress["status"], "completed");
        assert_eq!(progress["prepared"], 2);
        assert_eq!(progress["unresolved"], 0);
        let first = read_json(&output.join("input-000000/prepared.json"))
            .context("First receipt missing")?;
        let second = read_json(&output.join("input-000001/prepared.json"))
            .context("Second receipt missing")?;
        let batch = read_json(&output.join("preparation.json")).context("Batch receipt missing")?;
        assert_eq!(first["producer_sha256"], batch["producer_sha256"]);
        assert_eq!(second["producer_sha256"], batch["producer_sha256"]);
        assert_eq!(
            first["measurements"]["excel_pid"],
            second["measurements"]["excel_pid"]
        );
        assert_eq!(second["measurements"]["worker_jobs_completed"], 2);
        assert_eq!(first["source_sha256"], second["source_sha256"]);
        for receipt in [&first, &second] {
            assert_eq!(receipt["artifacts"].as_object().unwrap().len(), 3);
            assert!(matches!(
                receipt["native_result"]["status"].as_str(),
                Some("optimization_confirmed" | "no_optimization_needed")
            ));
        }
        for directory in ["input-000000", "input-000001"] {
            let receipt = read_json(&output.join(directory).join("prepared.json")).unwrap();
            assert_eq!(
                super::super::super::artifact_cache::hash_file(
                    &output.join(directory).join("baseline.xlsx")
                )?,
                receipt["source_sha256"]
            );
        }
        assert_eq!(
            read_json(&output.join("workers/status.json")).unwrap()["stopped"],
            true
        );
        for worker in fs::read_dir(output.join("workers"))? {
            let identity = worker?.path().join("identity.json");
            if let Some(identity) = read_json(&identity) {
                assert_eq!(
                    crate::native_process::process_alive(
                        identity["pid"].as_i64().unwrap(),
                        identity["started_ms"].as_i64().unwrap()
                    ),
                    Some(false)
                );
            }
        }
        Ok(())
    }
}
