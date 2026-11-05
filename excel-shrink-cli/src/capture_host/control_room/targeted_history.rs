//! Archive terminal display mirrors while retaining their original bytes and all run evidence.
use super::SNAPSHOT_LIMIT;
use anyhow::{ensure, Result};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

// The UI displays 100 completed attempts. Keep an additional buffer for readers
// and publishers completing concurrently.
const FINISHED_MIRRORS_TO_KEEP: usize = 200;

/// Maintain the display registry independently of scientific scheduling and HTTP queries.
pub(super) async fn maintain(root: PathBuf) {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let root = root.clone();
        match tokio::task::spawn_blocking(move || archive_finished(&root, FINISHED_MIRRORS_TO_KEEP))
            .await
        {
            Ok(Ok(0)) => {}
            Ok(Ok(count)) => tracing::info!(count, "Archived completed comparison display mirrors"),
            Ok(Err(error)) => tracing::warn!(%error, "Cannot maintain comparison display history"),
            Err(error) => tracing::warn!(%error, "Comparison display maintenance task failed"),
        }
    }
}

struct Mirror {
    path: PathBuf,
    finished_at: i64,
    archivable: bool,
}

/// Move only old, closed terminal snapshots into a separate history directory.
/// Unique sweep directories preserve earlier archived versions if a publisher retries.
fn archive_finished(root: &Path, keep: usize) -> Result<usize> {
    let registry = root.join("targeted-comparisons");
    let entries = match fs::read_dir(&registry) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut mirrors = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext| ext != "json")
        {
            continue;
        }
        let path = entry.path();
        match read_mirror(&path) {
            Ok(snapshot) => {
                if let Some(finished_at) = snapshot["finished_at"].as_i64() {
                    mirrors.push(Mirror {
                        archivable: closed_terminal(&path, &snapshot),
                        path,
                        finished_at,
                    });
                }
            }
            Err(error) => {
                tracing::warn!(path=%path.display(), %error, "Keeping unreadable comparison mirror")
            }
        }
    }
    mirrors.sort_by(|a, b| {
        b.finished_at
            .cmp(&a.finished_at)
            .then_with(|| b.path.cmp(&a.path))
    });
    let mut archive = None;
    let mut moved = 0;
    for mirror in mirrors
        .into_iter()
        .skip(keep)
        .filter(|mirror| mirror.archivable)
    {
        // Recheck before moving. An unfinished or replaced snapshot remains visible.
        let snapshot = match read_mirror(&mirror.path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(path=%mirror.path.display(), %error, "Keeping changed or unavailable comparison mirror");
                continue;
            }
        };
        if snapshot["finished_at"].as_i64() != Some(mirror.finished_at)
            || !closed_terminal(&mirror.path, &snapshot)
        {
            continue;
        }
        let destination = match &archive {
            Some(directory) => directory,
            None => {
                let directory = root.join("targeted-comparison-history").join(format!(
                    "{}-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                    uuid::Uuid::new_v4()
                ));
                fs::create_dir_all(&directory)?;
                archive.insert(directory)
            }
        };
        match fs::rename(
            &mirror.path,
            destination.join(mirror.path.file_name().unwrap()),
        ) {
            Ok(()) => moved += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path=%mirror.path.display(), %error, "Keeping comparison mirror after archive failure")
            }
        }
    }
    Ok(moved)
}

/// Require an owned mirror name, a terminal state and confirmed closure of every PID.
fn closed_terminal(path: &Path, snapshot: &Value) -> bool {
    let Some(id) = snapshot["run_id"].as_str() else {
        return false;
    };
    path.file_name()
        .is_some_and(|name| name == format!("{id}.json").as_str())
        && snapshot["finished_at"].as_i64().is_some()
        && matches!(
            snapshot["status"].as_str(),
            Some(
                "completed"
                    | "completed_with_limitations"
                    | "failed"
                    | "interrupted"
                    | "artifacts_prepared"
                    | "skipped"
            )
        )
        && snapshot["lanes"].as_array().is_some_and(|lanes| {
            lanes.iter().all(|lane| {
                lane["pid"].is_null()
                    || (lane["lifecycle"] == "closed" && lane["process_alive"] == false)
            })
        })
}

fn read_mirror(path: &Path) -> Result<Value> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(SNAPSHOT_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= SNAPSHOT_LIMIT,
        "Comparison mirror exceeds metadata read limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(root: &Path, id: &str, finished: Option<i64>) -> Result<PathBuf> {
        let registry = root.join("targeted-comparisons");
        fs::create_dir_all(&registry)?;
        let path = registry.join(format!("{id}.json"));
        fs::write(
            &path,
            json!({"run_id":id,"finished_at":finished,
            "status":"completed_with_limitations","lanes":[{
                "pid":12,"lifecycle":"closed","process_alive":false
            }]})
            .to_string(),
        )?;
        Ok(path)
    }

    #[test]
    fn archive_preserves_bytes_and_evidence_and_keeps_recent_and_unfinished_attempts() -> Result<()>
    {
        let directory =
            std::env::temp_dir().join(format!("mirror-history-{}", uuid::Uuid::new_v4()));
        let root = directory.as_path();
        let mut originals = Vec::new();
        for index in 1..=5 {
            let path = snapshot(root, &format!("run-{index}"), Some(index))?;
            originals.push((path.clone(), fs::read(path)?));
        }
        let active = snapshot(root, "active", None)?;
        let unconfirmed = snapshot(root, "unconfirmed", Some(0))?;
        let mut value = read_mirror(&unconfirmed)?;
        value["lanes"][0]["process_alive"] = json!(true);
        fs::write(&unconfirmed, value.to_string())?;
        let evidence = root.join("evidence");
        fs::create_dir_all(&evidence)?;
        fs::write(evidence.join("report.json"), b"authoritative report")?;
        assert_eq!(archive_finished(root, 2)?, 3);
        assert!(active.exists() && unconfirmed.exists());
        assert!(originals[3].0.exists() && originals[4].0.exists());
        let directory = fs::read_dir(root.join("targeted-comparison-history"))?
            .next()
            .unwrap()?
            .path();
        for (path, bytes) in originals.iter().take(3) {
            assert!(!path.exists());
            assert_eq!(&fs::read(directory.join(path.file_name().unwrap()))?, bytes);
        }
        assert_eq!(
            fs::read(evidence.join("report.json"))?,
            b"authoritative report"
        );
        assert_eq!(archive_finished(root, 2)?, 0);
        fs::remove_dir_all(&directory)?;
        Ok(())
    }

    #[test]
    fn unknown_or_unconfirmed_mirrors_remain_in_place() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("mirror-history-guards-{}", uuid::Uuid::new_v4()));
        let root = directory.as_path();
        let path = snapshot(root, "unknown", Some(0))?;
        let original = read_mirror(&path)?;
        for value in [
            json!({"status":"cleanup_unconfirmed"}),
            json!({"run_id":"different"}),
            json!({"lanes":[{"pid":12,"lifecycle":"closed","process_alive":null}]}),
        ] {
            let mut snapshot = original.clone();
            snapshot
                .as_object_mut()
                .unwrap()
                .extend(value.as_object().unwrap().clone());
            fs::write(&path, snapshot.to_string())?;
            assert_eq!(archive_finished(root, 0)?, 0);
            assert!(path.exists());
        }
        fs::write(&path, b"{incomplete")?;
        assert_eq!(archive_finished(root, 0)?, 0);
        assert!(path.exists());
        fs::remove_dir_all(&directory)?;
        Ok(())
    }
}
