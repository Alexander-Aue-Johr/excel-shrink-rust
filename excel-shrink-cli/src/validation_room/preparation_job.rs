//! A native preparation job outlives the browser request and retains frozen inputs and outcomes.
use super::*;
use serde::Serialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    source_manifest: Option<PathBuf>,
    source_directory: Option<PathBuf>,
    corpus_profile: Option<String>,
    output: PathBuf,
    workers: usize,
    reuse_from: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlRequest {
    job_id: String,
    workers: usize,
}

/// Adjust only the explicitly selected live preparation; busy sessions finish their workbook.
pub(super) async fn control(
    State(room): State<Arc<Room>>,
    Json(request): Json<ControlRequest>,
) -> ApiResult {
    let current = room
        .preparation_job
        .lock()
        .map_err(|_| anyhow::anyhow!("Preparation job lock poisoned"))?;
    let job = current
        .as_ref()
        .context("No preparation job is available")?;
    set_workers(job, &request)?;
    Ok(Json(
        json!({"accepted":true,"job_id":job.id,"workers":request.workers}),
    ))
}

fn set_workers(job: &Job, request: &ControlRequest) -> Result<()> {
    ensure!(request.workers > 0, "Preparation workers must be positive");
    ensure!(job.id == request.job_id, "Preparation job identity changed");
    ensure!(
        job.status == "running",
        "Preparation job is already terminal"
    );
    let path = job.output.join("control.json");
    ensure!(
        path.is_file(),
        "Preparation is still initializing; retry shortly"
    );
    crate::regression::artifact_cache::atomic_write(
        &path,
        &serde_json::to_vec(&json!({"version":1,"workers":request.workers}))?,
    )?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Job {
    version: u32,
    id: String,
    status: String,
    output: PathBuf,
    source_manifest: PathBuf,
    receipt: PathBuf,
    source_manifest_sha256: String,
    workers: usize,
    reuse_from: Option<PathBuf>,
    total: usize,
    owner_pid: u32,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
}

pub(super) async fn start(
    State(room): State<Arc<Room>>,
    Json(request): Json<StartRequest>,
) -> ApiResult {
    let reservation = reserve(&room)?;
    let runtime = tokio::runtime::Handle::current();
    // Admission and launch stay in one background operation if HTTP disconnects during freezing.
    tokio::task::spawn_blocking(move || -> ApiResult {
        let job = freeze_job(request)?;
        *room
            .preparation_job
            .lock()
            .map_err(|_| anyhow::anyhow!("Preparation job lock poisoned"))? = Some(job.clone());
        let accepted = serde_json::to_value(&job).map_err(anyhow::Error::from)?;
        runtime.spawn(async move {
            let _reservation = reservation;
            let parameters = job.clone();
            let outcome = tokio::task::spawn_blocking(move || -> Result<()> {
                ensure!(
                    crate::regression::artifact_cache::hash_file(&parameters.source_manifest)?
                        == parameters.source_manifest_sha256,
                    "Frozen preparation selection changed"
                );
                crate::regression::prepare_performance_manifest(
                    parameters.source_manifest,
                    parameters.output,
                    parameters.workers,
                    parameters.reuse_from,
                )
            })
            .await;
            let error = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(error) => Some(format!("Preparation worker stopped unexpectedly: {error}")),
            };
            let finished = finish_job(job, error);
            match room.preparation_job.lock() {
                Ok(mut current) => *current = Some(finished),
                Err(_) => eprintln!("Cannot update preparation job state: lock poisoned"),
            };
        });
        Ok(Json(json!({"accepted":true,"job":accepted})))
    })
    .await
    .map_err(anyhow::Error::from)?
}

/// Freeze absolute baseline paths so editing or moving the submitted manifest cannot change a job.
fn freeze_job(request: StartRequest) -> Result<Job> {
    ensure!(request.workers > 0, "Preparation workers must be positive");
    let inputs = match (&request.source_manifest, &request.source_directory) {
        (Some(path), None) => {
            ensure!(
                request.corpus_profile.is_none(),
                "A corpus profile applies to directory inputs"
            );
            crate::validation_preparation::read(path)?
        }
        (None, Some(path)) => crate::validation_preparation::directory_inputs(
            path,
            request.corpus_profile.as_deref(),
        )?,
        _ => anyhow::bail!("Choose either an XLSX directory or a source manifest"),
    };
    let output = std::path::absolute(request.output)?;
    let parent = fs::canonicalize(
        output
            .parent()
            .context("Preparation output has no parent")?,
    )?;
    let output = parent.join(
        output
            .file_name()
            .context("Choose a preparation output directory")?,
    );
    ensure!(
        !output.exists(),
        "Choose a new preparation output directory"
    );
    let id = uuid::Uuid::new_v4().to_string();
    let reuse_from = request.reuse_from.map(fs::canonicalize).transpose()?;
    let jobs_root = parent.join(".preparation-jobs");
    let directory = jobs_root.join(&id);
    fs::create_dir_all(&jobs_root)?;
    fs::create_dir(&directory)?;
    let source_manifest = directory.join("sources.json");
    let selected: Vec<_> = inputs
        .iter()
        .map(|input| {
            json!({
                "source_id":input.source_id,"name":input.name,"baseline":input.baseline,
                "baseline_sha256":input.expected_sha256,"raw_source":input.raw_source,
            })
        })
        .collect();
    crate::regression::artifact_cache::atomic_write(
        &source_manifest,
        &serde_json::to_vec_pretty(&json!({"version":1,"inputs":selected}))?,
    )?;
    let source_manifest_sha256 = crate::regression::artifact_cache::hash_file(&source_manifest)?;
    let job = Job {
        version: 1,
        id,
        status: "running".into(),
        output,
        source_manifest,
        receipt: directory.join("job.json"),
        source_manifest_sha256,
        workers: request.workers,
        reuse_from,
        total: inputs.len(),
        owner_pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        error: None,
    };
    write_job(&job)?;
    Ok(job)
}

fn finish_job(mut job: Job, error: Option<String>) -> Job {
    job.status = if error.is_some() {
        "failed"
    } else {
        "completed"
    }
    .into();
    job.error = error;
    job.finished_at = Some(chrono::Utc::now().to_rfc3339());
    if let Err(error) = write_job(&job) {
        job.status = "failed".into();
        job.error = Some(format!(
            "{}. Terminal job receipt could not be written: {error:#}",
            job.error
                .as_deref()
                .unwrap_or("Preparation returned successfully")
        ));
        eprintln!("{}", job.error.as_deref().unwrap());
    }
    job
}

fn write_job(job: &Job) -> Result<()> {
    crate::regression::artifact_cache::atomic_write(&job.receipt, &serde_json::to_vec_pretty(job)?)
}

pub(super) async fn snapshot(room: &Room) -> Result<Value> {
    let job = room
        .preparation_job
        .lock()
        .map_err(|_| anyhow::anyhow!("Preparation job lock poisoned"))?
        .clone();
    let Some(job) = job else {
        return Ok(Value::Null);
    };
    let progress_path = job.output.join("progress.json");
    let mut result = serde_json::to_value(&job)?;
    match read_small_json(&progress_path, 64 * 1024).await {
        Ok(progress) => result["progress"] = progress,
        Err(error) if progress_path.exists() => {
            result["progress_error"] = json!(format!("{error:#}"))
        }
        Err(_) => result["progress"] = Value::Null,
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_target_updates_require_matching_active_job() -> Result<()> {
        let (_, request) = request()?;
        let mut job = freeze_job(request)?;
        fs::create_dir(&job.output)?;
        let path = job.output.join("control.json");
        fs::write(&path, br#"{"version":1,"workers":2}"#)?;
        let mut control = ControlRequest {
            job_id: job.id.clone(),
            workers: 48,
        };
        set_workers(&job, &control)?;
        let saved = fs::read(&path)?;
        let value: Value = serde_json::from_slice(&saved)?;
        assert_eq!(value["workers"], 48);
        control.workers = 0;
        assert!(set_workers(&job, &control).is_err());
        control.workers = 3;
        control.job_id = "stale-job".into();
        assert!(set_workers(&job, &control).is_err());
        control.job_id = job.id.clone();
        job.status = "completed".into();
        assert!(set_workers(&job, &control).is_err());
        assert_eq!(fs::read(&path)?, saved);
        Ok(())
    }

    fn request() -> Result<(PathBuf, StartRequest)> {
        let root =
            std::env::temp_dir().join(format!("preparation-web-job-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let manifest = root.join("selected.json");
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({"version":1,"inputs":[{
                "source_id":"corpus:7:original.xls","name":"original.xls","baseline":"baseline.xlsx",
                "baseline_sha256":"a".repeat(64),"raw_source":{"sha256":"b".repeat(64),"format":"xls"}
            }]}))?,
        )?;
        Ok((
            root.clone(),
            StartRequest {
                source_manifest: Some(manifest),
                source_directory: None,
                corpus_profile: None,
                output: root.join("output"),
                workers: 32,
                reuse_from: None,
            },
        ))
    }

    #[test]
    fn job_freezes_selection_and_persists_independent_worker_settings() -> Result<()> {
        let (root, request) = request()?;
        let submitted = request.source_manifest.clone().unwrap();
        let job = freeze_job(request)?;
        assert!(!job.output.exists());
        assert_eq!(job.workers, 32);
        assert_eq!(job.total, 1);
        let inputs = crate::validation_preparation::read(&job.source_manifest)?;
        assert_eq!(inputs[0].source_id, "corpus:7:original.xls");
        assert_eq!(
            inputs[0].baseline,
            fs::canonicalize(&root)?.join("baseline.xlsx")
        );
        assert_eq!(
            inputs[0].expected_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
        fs::write(submitted, b"edited after submission")?;
        assert_eq!(
            crate::regression::artifact_cache::hash_file(&job.source_manifest)?,
            job.source_manifest_sha256
        );
        let receipt = job.receipt.clone();
        let terminal = finish_job(job, Some("fixture native failure".into()));
        assert_eq!(terminal.status, "failed");
        let saved: Value = serde_json::from_slice(&fs::read(receipt)?)?;
        assert_eq!(saved["error"], "fixture native failure");
        assert!(saved["finished_at"].is_string());
        Ok(())
    }

    #[test]
    fn invalid_workers_and_existing_outputs_fail_before_job_creation() -> Result<()> {
        let (root, mut request) = request()?;
        request.workers = 0;
        assert!(freeze_job(request).is_err());
        assert!(!root.join(".preparation-jobs").exists());
        let (root, request) = self::request()?;
        fs::create_dir(&request.output)?;
        assert!(freeze_job(request).is_err());
        assert!(!root.join(".preparation-jobs").exists());
        Ok(())
    }

    #[test]
    fn directory_selection_is_frozen_and_ambiguous_sources_are_rejected() -> Result<()> {
        let (root, mut request) = request()?;
        request.source_directory = Some(root.clone());
        assert!(freeze_job(request).is_err());
        assert!(!root.join(".preparation-jobs").exists());
        let (root, mut request) = self::request()?;
        fs::write(root.join("baseline.xlsx"), b"original bytes")?;
        request.source_manifest = None;
        request.source_directory = Some(root.clone());
        let job = freeze_job(request)?;
        let inputs = crate::validation_preparation::read(&job.source_manifest)?;
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].source_id, "directory:baseline.xlsx");
        assert_eq!(
            inputs[0].expected_sha256,
            Some(crate::validation_sources::hash_file(
                &root.join("baseline.xlsx")
            )?)
        );
        assert_eq!(inputs[0].raw_source.as_ref().unwrap().format, "xlsx");
        Ok(())
    }
}
