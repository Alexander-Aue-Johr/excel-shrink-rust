//! Prepare independent, attributable optimization artifacts with continuously reused Excel.
use super::*;
use crate::validation_preparation::{self, Input};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

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
    fs::create_dir(&args.output)?;
    let output = fs::canonicalize(&args.output)?;
    let producer = super::super::artifact_cache::hash_file(&std::env::current_exe()?)?;
    let mut pool = Pool::new_performance(output.join("workers"))?;
    let pool_root = pool.root.clone();
    let next = AtomicUsize::new(0);
    let outcomes = Mutex::new(Vec::new());
    let count = args.workers.min(inputs.len());
    let pool_result = std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = (0..count)
            .map(|_| {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(input) = inputs.get(index) else {
                        break;
                    };
                    let directory = output.join(format!("input-{index:06}"));
                    let result = prepare(&pool_root, input, &directory);
                    let outcome = json!({
                        "index": index,
                        "source_id": input.source_id,
                        "name": input.name,
                        "input": input.baseline,
                        "directory": directory,
                        "status": if result.is_ok() { "prepared" } else { "failed" },
                        "error": result.err().map(|error| format!("{error:#}"))
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
                })
            })
            .collect();
        while handles.iter().any(|handle| !handle.is_finished()) {
            if let Err(error) = pool.reconcile(count, 512) {
                pool.abort(&error)?;
                return Err(error);
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
            "workers": count,
            "requested_workers": args.workers,
            "producer_sha256": producer,
            "selection_sha256": selection_hash,
            "identity_scope": if args.source_manifest.is_some() { "source-manifest" } else { "explicit-file-list" },
            "manager_error": pool_result.as_ref().err().map(|error| format!("{error:#}")),
            "visual_validation": "pending",
            "total": inputs.len(),
            "failed": failed,
            "inputs": outcomes
        }),
    )?;
    pool_result?;
    anyhow::ensure!(
        failed == 0,
        "{failed} Check Performance preparations failed; inspect preparation.json"
    );
    Ok(())
}

fn prepare(root: &Path, source: &Input, directory: &Path) -> Result<()> {
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
    for name in [
        "original-saved.xlsx",
        "before-optimization.xlsx",
        "after-optimization.xlsx",
    ] {
        let path = directory.join(name);
        artifacts.insert(
            name,
            json!({
                "bytes": fs::metadata(&path)?.len(),
                "sha256": super::super::artifact_cache::hash_file(&path)?
            }),
        );
    }
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
    write_json(
        &directory.join("prepared.json"),
        &json!({
            "version": 1,
            "operation": "check-performance",
            "source": input,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_workers_before_creating_output() {
        let output = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let error = run(Args {
            input: vec![],
            source_manifest: None,
            output: output.clone(),
            workers: 0,
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
        let error = prepare(&pool, &input, &root.join("result")).unwrap_err();
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
        })?;
        let first = read_json(&output.join("input-000000/prepared.json"))
            .context("First receipt missing")?;
        let second = read_json(&output.join("input-000001/prepared.json"))
            .context("Second receipt missing")?;
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
