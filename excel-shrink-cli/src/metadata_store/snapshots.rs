//! Coalesced comparison snapshots and compatible file references.
use super::Store;
use anyhow::{ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// One comparison owns its database and coalesces transient snapshots. Terminal
/// state is committed before the portable JSON exports are published.
pub(crate) struct SnapshotPublisher {
    store: Store,
    database: PathBuf,
    last_published: Option<Instant>,
    references_published: bool,
}

impl SnapshotPublisher {
    pub(crate) fn new(database: PathBuf) -> Result<Self> {
        let database = std::path::absolute(database)?;
        Ok(Self {
            store: Store::open(&database)?,
            database,
            last_published: None,
            references_published: false,
        })
    }

    /// Retain at most one transient state per second, without delaying a final result.
    pub(crate) fn publish(
        &mut self,
        value: &mut serde_json::Value,
        paths: &[PathBuf],
    ) -> Result<()> {
        let terminal = value["finished_at"].is_i64();
        if !terminal
            && self
                .last_published
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return Ok(());
        }
        let bytes = serde_json::to_vec(value)?;
        self.store.put("snapshot.json", &bytes)?;
        let reference = serde_json::to_vec(&serde_json::json!({
            "_metadata_store": self.database,
            "capture_success_sequence": value["capture_success_sequence"],
            "capture_progress_sequence": value["capture_progress_sequence"]
        }))?;
        for path in paths {
            // The small local reference carries watchdog counters. Display
            // references are immutable while the same comparison is running.
            if terminal || path == &paths[0] || !self.references_published {
                let result = crate::regression::artifact_cache::atomic_write(
                    path,
                    if terminal && path == &paths[0] {
                        &bytes
                    } else {
                        &reference
                    },
                );
                if path == &paths[0] {
                    result?;
                } else if let Err(error) = result {
                    value["publication_errors"]
                        .as_array_mut()
                        .context("Missing publication errors")?
                        .push(serde_json::json!(format!(
                            "Publish {}: {error:#}",
                            path.display()
                        )));
                }
            }
        }
        if value["publication_errors"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            self.store
                .put("snapshot.json", &serde_json::to_vec(value)?)?;
        } else {
            self.references_published = true;
        }
        self.last_published = Some(Instant::now());
        Ok(())
    }
}

/// Read a committed snapshot through a small compatibility reference. Unknown
/// JSON fields are never interpreted as database paths unless the marker exists.
pub(crate) fn linked_snapshot(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>> {
    if fs::metadata(path)?.len() > 4096 {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(None);
    };
    let Some(database) = value.get("_metadata_store") else {
        return Ok(None);
    };
    let database = PathBuf::from(
        database
            .as_str()
            .context("Invalid SQLite metadata reference")?,
    );
    ensure!(
        database.is_absolute()
            && database
                .file_name()
                .is_some_and(|name| name == "telemetry.sqlite"),
        "Invalid SQLite snapshot database"
    );
    let store = Store::open_readonly(&database)?;
    let size: i64 = store.connection.query_row(
        "SELECT length(payload) FROM documents WHERE path='snapshot.json'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        size >= 0 && size as u64 <= maximum,
        "SQLite snapshot exceeds metadata limit"
    );
    Ok(Some(
        store
            .get("snapshot.json")?
            .context("Missing SQLite snapshot")?,
    ))
}

pub(crate) fn read_snapshot(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    if let Some(bytes) = linked_snapshot(path, maximum)? {
        return Ok(bytes);
    }
    ensure!(
        fs::metadata(path)?.len() <= maximum,
        "Snapshot exceeds metadata limit"
    );
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "Snapshot exceeds metadata limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sqlite_snapshots_coalesce_updates_but_export_terminal_state() -> Result<()> {
        let root = std::env::temp_dir().join(format!("metadata-snapshot-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let paths = vec![root.join("telemetry.json"), root.join("mirror.json")];
        let mut publisher = SnapshotPublisher::new(root.join("telemetry.sqlite"))?;
        let mut value = serde_json::json!({"finished_at":null,"publication_errors":[],"capture_success_sequence":1,"capture_progress_sequence":1});
        publisher.publish(&mut value, &paths)?;
        let first = read_snapshot(&paths[1], 4096)?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&first)?["capture_success_sequence"],
            1
        );
        value["capture_success_sequence"] = serde_json::json!(2);
        publisher.publish(&mut value, &paths)?;
        assert_eq!(read_snapshot(&paths[1], 4096)?, first);
        assert!(read_snapshot(&paths[1], 1).is_err());
        value["finished_at"] = serde_json::json!(12345);
        publisher.publish(&mut value, &paths)?;
        let terminal: serde_json::Value = serde_json::from_slice(&fs::read(&paths[0])?)?;
        assert_eq!(terminal["capture_success_sequence"], 2);
        assert!(terminal.get("_metadata_store").is_none());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&read_snapshot(&paths[1], 4096)?)?
                ["capture_success_sequence"],
            2
        );
        drop(publisher);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
