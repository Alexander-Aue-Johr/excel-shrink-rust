//! Embedded SQLite storage shared by migration and diagnostic publishers.
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path},
    time::Duration,
};

mod migration;
mod session;
mod snapshots;
pub(crate) use migration::run;
pub(crate) use session::SessionWriter;
pub(crate) use snapshots::{linked_snapshot, read_snapshot, SnapshotPublisher};

pub(crate) struct Store {
    connection: Connection,
}

impl Store {
    /// Open a local metadata database with bounded lock waits and durable commits.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                path TEXT PRIMARY KEY NOT NULL,
                payload BLOB NOT NULL,
                sha256 BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );",
        )?;
        Ok(Self { connection })
    }

    fn open_readonly(path: &Path) -> Result<Self> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { connection })
    }

    /// Store exact input bytes. JSON formatting and unknown fields survive export.
    pub(crate) fn put(&self, path: &str, bytes: &[u8]) -> Result<()> {
        validate_key(path)?;
        let digest = Sha256::digest(bytes);
        self.connection.execute(
            "INSERT INTO documents(path, payload, sha256) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET payload=excluded.payload, sha256=excluded.sha256",
            params![path, bytes, digest.as_slice()],
        )?;
        Ok(())
    }

    /// Read only bytes whose stored checksum matches their content.
    pub(crate) fn get(&self, path: &str) -> Result<Option<Vec<u8>>> {
        validate_key(path)?;
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT payload, sha256 FROM documents WHERE path=?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((bytes, digest)) => {
                ensure!(
                    Sha256::digest(&bytes).as_slice() == digest,
                    "Metadata checksum differs for {path}"
                );
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }
}

/// Reject absolute paths and traversal before any export can touch the filesystem.
fn validate_key(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty() && !key.contains('\\') && !key.contains(':'),
        "Invalid metadata path"
    );
    ensure!(
        Path::new(key)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Invalid metadata path"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn aborted_transaction_keeps_the_previous_snapshot() -> Result<()> {
        let root = std::env::temp_dir().join(format!("metadata-rollback-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let path = root.join("state.sqlite");
        {
            let store = Store::open(&path)?;
            store.put("snapshot.json", b"old")?;
            store.connection.execute_batch("BEGIN IMMEDIATE")?;
            store.put("snapshot.json", b"uncommitted")?;
        }
        let store = Store::open_readonly(&path)?;
        assert_eq!(
            store.get("snapshot.json")?.as_deref(),
            Some(b"old".as_slice())
        );
        drop(store);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn corrupt_payload_cannot_be_exported_as_valid_metadata() -> Result<()> {
        let store = Store::open(Path::new(":memory:"))?;
        store.put("snapshot.json", b"original")?;
        store
            .connection
            .execute("UPDATE documents SET payload=?1", [b"corrupt".as_slice()])?;
        assert!(store.get("snapshot.json").is_err());
        Ok(())
    }
}
