//! Durable browser requests are consumed only by the owning study coordinator.
use super::*;
use std::collections::BTreeMap;

pub(super) struct Request {
    path: PathBuf,
    pub value: Value,
    pub job: Comparison,
}

impl Request {
    pub fn update(&mut self, status: &str, message: &str) -> Result<()> {
        self.value["status"] = json!(status);
        self.value["message"] = json!(message);
        self.value["updated_unix_ms"] = json!(provenance::now_ms());
        control_room::write(&self.path, &self.value)
    }
}

pub(super) fn eligible(source: &Source, records: &[Value]) -> BTreeMap<String, Comparison> {
    let positions: BTreeMap<_, _> = records
        .iter()
        .enumerate()
        .filter_map(|(position, record)| record["identity"].as_str().map(|id| (id, position)))
        .collect();
    source
        .members
        .iter()
        .filter(|(index, _)| source.selection.contains(*index))
        .filter_map(|(index, name)| {
            let id = identity(source, *index, name);
            let position = *positions.get(id.as_str())?;
            Some((
                control_room::group_id(&id),
                Comparison {
                    index: *index,
                    id,
                    position,
                },
            ))
        })
        .collect()
}

fn invalid_reason(command: &Value, queue: &Value, eligible: bool) -> Option<&'static str> {
    if command["type"] != "restart_group" {
        return Some("Unknown control command.");
    }
    if command["run_id"] != queue["run_id"] {
        return Some("The request belongs to an earlier run.");
    }
    if !eligible {
        return Some("This workbook is outside the current corpus/size selection.");
    }
    let group = queue["groups"]
        .as_array()?
        .iter()
        .find(|g| g["id"] == command["group_id"]);
    let Some(group) = group else {
        return Some("The workbook is not in this queue.");
    };
    if group["attempt_id"] != command["attempt_id"] {
        return Some("The displayed attempt has already been replaced; refresh before restarting.");
    }
    None
}

pub(super) fn read_pending(
    room: &Path,
    queue: &Value,
    eligible: &BTreeMap<String, Comparison>,
) -> Result<Vec<Request>> {
    let directory = room.join("commands");
    fs::create_dir_all(&directory)?;
    let mut requests = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(mut value) = control_room::read_json(&path) else {
            continue;
        };
        if value["status"] != "pending" {
            continue;
        }
        let job = value["group_id"].as_str().and_then(|id| eligible.get(id));
        if let Some(reason) = invalid_reason(&value, queue, job.is_some()) {
            value["status"] = json!("rejected");
            value["message"] = json!(reason);
            value["updated_unix_ms"] = json!(provenance::now_ms());
            control_room::write(&path, &value)?;
            continue;
        }
        requests.push(Request {
            path,
            value,
            job: job.context("restart has no eligible workbook")?.clone(),
        });
    }
    requests.sort_by_key(|request| request.value["requested_unix_ms"].as_u64().unwrap_or(0));
    Ok(requests)
}

pub(super) fn move_to_front(pending: &mut VecDeque<Comparison>, job: &Comparison) {
    pending.retain(|queued| queued.id != job.id);
    pending.push_front(job.clone());
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poll(
    room: &Path,
    queue: &mut Value,
    eligible: &BTreeMap<String, Comparison>,
    active: &[Active],
    pending: &mut VecDeque<Comparison>,
    records: &mut [Value],
    mut cleanup: Option<&mut batch_cleanup::Batch>,
    queued: &mut BTreeMap<String, Vec<Request>>,
) -> Result<()> {
    for mut request in read_pending(room, queue, eligible)? {
        let id = control_room::group_id(&request.job.id);
        let working = active
            .iter()
            .find(|a| a.job.id == request.job.id && !a.finished);
        if let Some(group) = working {
            group.participant.request_restart()?;
            if let Some(batch) = cleanup.as_deref_mut() {
                batch.close_admission();
            }
        } else {
            let record = &mut records[request.job.position];
            control_room::archive_record(room, &request.job.id, record)?;
            record["complete"] = json!(false);
            record["status"] = json!("restart_pending");
            write_record(
                &record_path(
                    room.parent().context("control room output missing")?,
                    &request.job.id,
                ),
                record,
            )?;
        }
        move_to_front(pending, &request.job);
        request.update(
            "queued",
            if working.is_some() {
                "Restart queued next; keeping Excel open until this wave's peers can close safely."
            } else {
                "Restart queued at the front of the selected corpus."
            },
        )?;
        if let Some(group) = queue["groups"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["id"] == id)
        {
            group["status"] = json!(if working.is_some() {
                "restart_waiting_cleanup"
            } else {
                "queued"
            });
        }
        queued
            .entry(request.job.id.clone())
            .or_default()
            .push(request);
    }
    Ok(())
}

pub(super) fn finish(requests: &mut [Request], record: &Value) -> Result<()> {
    for request in requests {
        request.value["result_status"] = record["status"].clone();
        request.value["replacement_attempt_id"] = record["control_room_attempt"]["id"].clone();
        request.update(
            if record["complete"] == true {
                "completed"
            } else {
                "superseded"
            },
            if record["complete"] == true {
                "Replacement comparison finished; see its result status."
            } else {
                "Replacement attempt was interrupted by another restart."
            },
        )?;
    }
    Ok(())
}

pub(super) fn begin_attempt(
    room: &Path,
    job: &Comparison,
    record: &mut Value,
    controls: &Controls,
    run: &Value,
) -> Result<String> {
    control_room::archive_record(room, &job.id, record)?;
    let attempt = provenance::new_uuid()?;
    record["control_room_attempt"] = json!({"id":attempt,"run_id":run["run_id"],
        "started_unix_ms":provenance::now_ms(),"settings":controls});
    let state = room
        .join("groups")
        .join(control_room::group_id(&job.id))
        .join("state.json");
    if state.is_file() {
        let previous = state.with_file_name(format!("previous-{attempt}.json"));
        fs::rename(&state, previous).context("archive previous group snapshot")?;
    }
    // Persist before starting any worker, so an interrupted launch is resumable.
    record["complete"] = json!(false);
    write_record(
        &record_path(
            room.parent().context("control room output missing")?,
            &job.id,
        ),
        record,
    )?;
    Ok(attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_attempts_and_unselected_workbooks_cannot_restart() {
        let queue = json!({"run_id":"current","groups":[{"id":"a","attempt_id":"new"}]});
        let mut command =
            json!({"type":"restart_group","run_id":"current","group_id":"a","attempt_id":"new"});
        assert!(invalid_reason(&command, &queue, true).is_none());
        assert!(
            invalid_reason(&command, &queue, false)
                .unwrap()
                .contains("selection")
        );
        command["attempt_id"] = json!("old");
        assert!(
            invalid_reason(&command, &queue, true)
                .unwrap()
                .contains("replaced")
        );
        command["run_id"] = json!("old");
        assert!(
            invalid_reason(&command, &queue, true)
                .unwrap()
                .contains("earlier run")
        );
    }

    #[test]
    fn a_restart_moves_one_copy_to_the_front() {
        let job = |index| Comparison {
            index,
            id: index.to_string(),
            position: index,
        };
        let mut pending = VecDeque::from([job(1), job(2), job(3)]);
        move_to_front(&mut pending, &job(2));
        move_to_front(&mut pending, &job(2));
        assert_eq!(
            pending.iter().map(|job| job.index).collect::<Vec<_>>(),
            [2, 1, 3]
        );
    }

    #[test]
    fn replacements_preserve_previous_record_and_freeze_settings() -> Result<()> {
        let output = artifact_cache::unique_path(&std::env::temp_dir(), "manual-attempt")?;
        let room = output.join("control-room");
        fs::create_dir_all(output.join("records"))?;
        let job = Comparison {
            index: 0,
            id: "example".into(),
            position: 0,
        };
        let mut record = json!({"complete":true,"results":["previous evidence"]});
        let controls: Controls = serde_json::from_value(json!({"target_groups":2,"paused":false,
            "open_timeout_seconds":60,"job_timeout_seconds":60,"equal_preview_width":512,
            "capture_tile_megapixels":4,"capture_batch_tiles":3,"hide_excel":true,
            "system_reserve_mib":3072}))?;
        let first = begin_attempt(
            &room,
            &job,
            &mut record,
            &controls,
            &json!({"run_id":"run"}),
        )?;
        assert_eq!(record["complete"], false);
        assert_eq!(
            record["control_room_attempt"]["settings"]["capture_tile_megapixels"],
            4
        );
        assert_eq!(
            record["control_room_attempt"]["settings"]["system_reserve_mib"],
            3072
        );
        let second = begin_attempt(
            &room,
            &job,
            &mut record,
            &controls,
            &json!({"run_id":"run"}),
        )?;
        assert_ne!(first, second);
        let history = room
            .join("groups")
            .join(control_room::group_id(&job.id))
            .join("history");
        let legacy = control_room::read_json(&history.join(format!(
            "record-{}.json",
            &artifact_cache::sha256(b"legacy")[..20]
        )))
        .unwrap();
        assert_eq!(legacy["complete"], true);
        assert_eq!(legacy["results"][0], "previous evidence");
        fs::remove_dir_all(output)?;
        Ok(())
    }
}
