//! Profile acquisition continues independently of the initiating browser request.

use super::*;
use serde::Serialize;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Inspect,
    Prepare,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    profile: String,
    operation: Operation,
    artifacts: PathBuf,
    cache: PathBuf,
    archive: Option<PathBuf>,
}

impl Request {
    fn validate(&self) -> Result<()> {
        let catalog = crate::validation_corpora::catalog()?;
        let selected = catalog["profiles"]
            .as_array()
            .context("Corpus catalog")?
            .iter()
            .find(|profile| profile["id"] == self.profile)
            .context("Choose a versioned corpus profile")?;
        ensure!(selected["acquisition_ready"] != false,
            "This source collection requires its extraction recipe and imported workbook manifest first");
        ensure!(
            self.artifacts.is_absolute(),
            "Choose an absolute originals directory"
        );
        ensure!(
            self.cache.is_absolute(),
            "Choose an absolute download cache directory"
        );
        ensure!(
            self.archive.as_ref().is_none_or(|path| path.is_absolute()),
            "Choose an absolute local archive path"
        );
        Ok(())
    }

    fn execute(&self, progress: &dyn Fn(Value)) -> Result<Value> {
        let mut result = match self.operation {
            Operation::Inspect => {
                crate::validation_corpora::inspect(&self.profile, &self.artifacts)?
            }
            Operation::Prepare => crate::validation_corpora::prepare_with_progress(
                &self.profile,
                &self.artifacts,
                &self.cache,
                self.archive.as_deref(),
                progress,
            )?,
        };
        // Keep polling bounded even when an entire corpus is missing.
        if let Some(Value::Object(fields)) = result.get_mut("inspection") {
            fields.remove("missing");
            fields.remove("changed");
        }
        if let Some(fields) = result.as_object_mut() {
            fields.remove("missing");
            fields.remove("changed");
        }
        Ok(result)
    }
}

#[derive(Serialize)]
pub(super) struct Job {
    id: String,
    request: Request,
    status: &'static str,
    progress: Value,
    result: Option<Value>,
    error: Option<String>,
}

pub(super) fn snapshot(room: &Room) -> Result<Value> {
    let job = room
        .corpus_job
        .lock()
        .map_err(|_| anyhow::anyhow!("Corpus job lock poisoned"))?;
    Ok(serde_json::to_value(&*job)?)
}

pub(super) async fn start(
    State(room): State<Arc<Room>>,
    Json(request): Json<Request>,
) -> ApiResult {
    request.validate()?;
    let reservation = reserve(&room)?;
    let id = uuid::Uuid::new_v4().to_string();
    {
        let mut current = room
            .corpus_job
            .lock()
            .map_err(|_| anyhow::anyhow!("Corpus job lock poisoned"))?;
        *current = Some(Job {
            id: id.clone(),
            request: request.clone(),
            status: "running",
            progress: json!({"phase":"checking_originals"}),
            result: None,
            error: None,
        });
    }
    let accepted = snapshot(&room)?;
    tokio::spawn(async move {
        let _reservation = reservation;
        let worker_room = room.clone();
        let result = tokio::task::spawn_blocking(move || {
            request.execute(&|progress| {
                let mut current = worker_room
                    .corpus_job
                    .lock()
                    .expect("Corpus progress lock poisoned");
                if let Some(job) = current.as_mut() {
                    job.progress = progress;
                }
            })
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|result| result);
        match room.corpus_job.lock() {
            Ok(mut current) => {
                if let Some(job) = current.as_mut() {
                    match result {
                        Ok(result) => {
                            job.status = "completed";
                            job.result = Some(result);
                        }
                        Err(error) => {
                            job.status = "failed";
                            job.error = Some(format!("{error:#}"));
                        }
                    }
                }
            }
            Err(error) => tracing::error!(%error, "Cannot record corpus job completion"),
        };
    });
    Ok(Json(json!({"job":accepted})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_source_collection_cannot_start_acquisition() {
        let root = std::env::temp_dir().join(format!("enron-pending-{}", uuid::Uuid::new_v4()));
        let request = Request {
            profile: "enron-edrm-v2".into(),
            operation: Operation::Prepare,
            artifacts: root.join("originals"),
            cache: root.join("cache"),
            archive: None,
        };
        assert!(request
            .validate()
            .unwrap_err()
            .to_string()
            .contains("extraction recipe"));
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn read_only_job_finishes_after_request_and_keeps_polling_compact() -> Result<()> {
        let root = std::env::temp_dir().join(format!("corpus-web-{}", uuid::Uuid::new_v4()));
        let room = Arc::new(Room {
            application: Application::DeltaSieve,
            port: 1234,
            token: "test".into(),
            config: Mutex::new(RunConfig::default()),
            progress: Arc::new(Mutex::new(LiveProgress::default())),
            active: AtomicBool::new(false),
            report: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
            picker: Mutex::new(()),
            study: Mutex::new(None),
            study_defaults: crate::validation_study::Configuration::default(),
            preparation_job: Mutex::new(None),
            corpus_job: Mutex::new(None),
        });
        let request = Request {
            profile: "datagov".into(),
            operation: Operation::Inspect,
            artifacts: root.join("originals"),
            cache: root.join("cache"),
            archive: None,
        };
        room.active.store(true, Ordering::Release);
        assert!(start(State(room.clone()), Json(request.clone()))
            .await
            .is_err());
        assert!(snapshot(&room)?.is_null());
        room.active.store(false, Ordering::Release);
        let accepted = start(State(room.clone()), Json(request)).await.unwrap().0;
        assert_eq!(accepted["job"]["status"], "running");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while room.active.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;
        let completed = snapshot(&room)?;
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["result"]["missing_count"], 9117);
        assert!(completed["result"].get("missing").is_none());
        assert!(serde_json::to_vec(&completed)?.len() < 2048);
        assert!(!root.exists());
        Ok(())
    }
}
