//! Local package operations share the tested CLI contracts and the room's exclusive reservation.
use super::*;
use crate::validation_preparation::package;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreviewRequest {
    package: PathBuf,
    source_manifest: PathBuf,
    #[serde(default)]
    offset: usize,
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportRequest {
    package: PathBuf,
    source_manifest: PathBuf,
    output: PathBuf,
    generation_choices: Option<PathBuf>,
    #[serde(default)]
    adopt_packaged_baseline: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportRequest {
    preparation: PathBuf,
    output: PathBuf,
}

pub(super) async fn preview(
    State(room): State<Arc<Room>>,
    Json(request): Json<PreviewRequest>,
) -> ApiResult {
    operation(room, move || {
        Ok(serde_json::to_value(package::preview(
            &request.package,
            &request.source_manifest,
            request.offset,
            request.limit,
        )?)?)
    })
    .await
}

pub(super) async fn import(
    State(room): State<Arc<Room>>,
    Json(request): Json<ImportRequest>,
) -> ApiResult {
    operation(room, move || {
        let options = package::ImportOptions::from_arguments(request.generation_choices.as_deref(), request.adopt_packaged_baseline)?;
        let count = package::materialize(&request.package, &request.source_manifest, &request.output, &options)?;
        let configuration_path = fs::canonicalize(request.output.join("compare.json"))?;
        let configuration = RunConfig::from_file(&configuration_path)?;
        Ok(json!({"imported":count,"configuration_path":configuration_path,"configuration":configuration}))
    }).await
}

pub(super) async fn export(
    State(room): State<Arc<Room>>,
    Json(request): Json<ExportRequest>,
) -> ApiResult {
    operation(room, move || {
        let header = package::export(&request.preparation, &request.output)?;
        Ok(json!({"exported":header.entries,"output":fs::canonicalize(request.output)?}))
    })
    .await
}

/// The file operation owns its reservation even if the browser disconnects.
async fn operation(
    room: Arc<Room>,
    work: impl FnOnce() -> Result<Value> + Send + 'static,
) -> ApiResult {
    let reservation = reserve(&room)?;
    let value = tokio::task::spawn_blocking(move || {
        let _reservation = reservation;
        work()
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Arc<Room> {
        Arc::new(Room {
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
        })
    }

    #[tokio::test]
    async fn session_preserves_corpus_prefill_without_starting_work() {
        let mut room = room();
        let configuration = &mut Arc::get_mut(&mut room).unwrap().study_defaults;
        configuration.name = "Destatis".into();
        configuration.partitions = 6;
        configuration.partition = 2;
        configuration.comparison_groups = 3;
        let response = session(State(room.clone())).await.unwrap().0;
        assert_eq!(response["study_defaults"]["name"], "Destatis");
        assert_eq!(response["study_defaults"]["partition"], 2);
        assert_eq!(response["study_defaults"]["partitions"], 6);
        assert_eq!(response["study_defaults"]["comparison_groups"], 3);
        assert!(!room.active.load(Ordering::Acquire));
        assert!(room.study.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn package_operations_release_the_room_after_success_and_failure() {
        let room = room();
        let result = operation(room.clone(), || Ok(json!({"verified":true})))
            .await
            .unwrap();
        assert_eq!(result.0["verified"], true);
        assert!(!room.active.load(Ordering::Acquire));
        assert!(operation(room.clone(), || anyhow::bail!("fixture failure"))
            .await
            .is_err());
        assert!(!room.active.load(Ordering::Acquire));
        let reservation = reserve(&room).unwrap();
        assert!(
            operation(room.clone(), || panic!("reserved work must not execute"))
                .await
                .is_err()
        );
        assert!(room.active.load(Ordering::Acquire));
        drop(reservation);
        assert!(!room.active.load(Ordering::Acquire));
    }

    #[test]
    fn package_requests_reject_unknown_fields() {
        assert!(serde_json::from_value::<PreviewRequest>(
            json!({"package":"p","source_manifest":"s","limit":100,"command":"unexpected"})
        )
        .is_err());
        let request: ImportRequest =
            serde_json::from_value(json!({"package":"p","source_manifest":"s","output":"new"}))
                .unwrap();
        assert!(!request.adopt_packaged_baseline);
        assert!(request.generation_choices.is_none());
    }
}
