//! One resident host owns diagnostic snapshots and ordered launcher events.
use super::Store;
use anyhow::{ensure, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

pub(crate) struct SessionWriter {
    store: Store,
    session: PathBuf,
    group_snapshot: Option<PathBuf>,
    references_published: bool,
}

impl SessionWriter {
    pub(crate) fn new(session: PathBuf, group_snapshot: Option<PathBuf>) -> Result<Self> {
        let session = std::path::absolute(session)?;
        let store = Store::open(&session.join("telemetry.sqlite"))?;
        store.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                sequence INTEGER PRIMARY KEY,
                id TEXT UNIQUE NOT NULL,
                payload BLOB NOT NULL
            );",
        )?;
        if store.get("initial-events.jsonl")?.is_none() {
            let legacy = match fs::read(session.join("launcher-events.jsonl")) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => return Err(error.into()),
            };
            store.put("initial-events.jsonl", &legacy)?;
        }
        Ok(Self {
            store,
            session,
            group_snapshot,
            references_published: false,
        })
    }

    /// Acknowledge only committed events. Retried event IDs never duplicate history.
    pub(crate) fn publish(&mut self, request: &Value) -> Result<usize> {
        let events = request["events"]
            .as_array()
            .context("Missing diagnostic event batch")?;
        ensure!(events.len() <= 256, "Diagnostic batch is too large");
        let transaction = self.store.connection.transaction()?;
        for event in events {
            let id = event["id"].as_str().context("Diagnostic event has no ID")?;
            ensure!(
                uuid::Uuid::parse_str(id).is_ok(),
                "Invalid diagnostic event ID"
            );
            let bytes = serde_json::to_vec(event)?;
            transaction.execute(
                "INSERT INTO events(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO NOTHING",
                params![id, bytes],
            )?;
            let stored: Vec<u8> =
                transaction.query_row("SELECT payload FROM events WHERE id=?1", [id], |row| {
                    row.get(0)
                })?;
            ensure!(
                stored == bytes,
                "Diagnostic event ID was reused with different content"
            );
        }
        if let Some(metrics) = request.get("metrics").filter(|value| value.is_object()) {
            let bytes = serde_json::to_vec(metrics)?;
            use sha2::{Digest, Sha256};
            transaction.execute(
                "INSERT INTO documents(path,payload,sha256) VALUES ('snapshot.json',?1,?2)
                 ON CONFLICT(path) DO UPDATE SET payload=excluded.payload,sha256=excluded.sha256",
                params![bytes, Sha256::digest(&bytes).as_slice()],
            )?;
        }
        transaction.commit()?;
        // Opening and resource observers in the existing PowerShell transport
        // read this file directly. Keep that contract until they use native reads.
        if let Some(metrics) = request.get("metrics").filter(|value| value.is_object()) {
            crate::regression::artifact_cache::atomic_write(
                &self.session.join("metrics.json"),
                &serde_json::to_vec(metrics)?,
            )?;
        }
        if !self.references_published && self.store.get("snapshot.json")?.is_some() {
            let marker = serde_json::to_vec(
                &json!({"_metadata_store":self.session.join("telemetry.sqlite")}),
            )?;
            if let Some(path) = &self.group_snapshot {
                crate::regression::artifact_cache::atomic_write(&path, &marker)?;
            }
            self.references_published = true;
        }
        if request["export"] == true {
            self.export()?;
        }
        Ok(events.len())
    }

    /// Export once at teardown so existing report collectors remain compatible.
    fn export(&self) -> Result<()> {
        use std::io::Write;
        let path = self.session.join("launcher-events.jsonl");
        let pending = self
            .session
            .join(format!(".events-export-{}", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(
            &self
                .store
                .get("initial-events.jsonl")?
                .context("Missing initial event history")?,
        )?;
        let mut statement = self
            .store
            .connection
            .prepare("SELECT payload FROM events ORDER BY sequence")?;
        let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        for row in rows {
            file.write_all(&row?)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
        drop(file);
        // The launcher is the only JSONL writer until this acknowledged export.
        crate::regression::artifact_cache::atomic_replace(&pending, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_retries_are_idempotent_and_conflicts_rollback() -> Result<()> {
        let root = std::env::temp_dir().join(format!("metadata-session-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        fs::write(root.join("launcher-events.jsonl"), b"{\"legacy\":true}\n")?;
        let mut writer = SessionWriter::new(root.clone(), None)?;
        let id = uuid::Uuid::new_v4().to_string();
        let request = json!({"events":[{"id":id,"event":"opened"}],"metrics":{"status":"running"}});
        writer.publish(&request)?;
        writer.publish(&request)?;
        assert!(writer
            .publish(&json!({"events":[{"id":id,"event":"different"}]}))
            .is_err());
        writer.publish(&json!({"events":[],"export":true}))?;
        let events = fs::read_to_string(root.join("launcher-events.jsonl"))?;
        assert_eq!(events.lines().count(), 2);
        assert!(events.contains("opened"));
        assert!(!events.contains("different"));
        let metrics: Value = serde_json::from_slice(&super::super::read_snapshot(
            &root.join("metrics.json"),
            4096,
        )?)?;
        assert_eq!(metrics["status"], "running");
        let compatibility: Value = serde_json::from_slice(&fs::read(root.join("metrics.json"))?)?;
        assert_eq!(compatibility["status"], "running");
        assert!(compatibility.get("_metadata_store").is_none());
        drop(writer);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
