//! Bounded, read-only reuse of parsed display snapshots. Scientific decisions
//! and durable publication remain with their existing owners.
use super::snapshots::read_object;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;

const MAX_ENTRIES: usize = 512;
const MAX_RETAINED_BYTES: usize = 32 * 1024 * 1024;
const REVALIDATE_AFTER: Duration = Duration::from_secs(30);

#[derive(Eq, PartialEq)]
struct Stamp {
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

impl Stamp {
    async fn read(path: &Path) -> Result<Self> {
        let metadata = tokio::fs::metadata(path).await?;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
            created: metadata.created().ok(),
        })
    }
}

struct Entry {
    stamp: Stamp,
    value: Arc<Value>,
    weight: usize,
    used: u64,
    read_at: Instant,
    sqlite: bool,
    encoded_bytes: u64,
}

#[derive(Default)]
struct Entries {
    values: HashMap<PathBuf, Entry>,
    retained_bytes: usize,
    clock: u64,
}

/// Readers share parsed metadata, with one loader preventing concurrent copies
/// of a large corpus index. Errors never return a formerly valid cached value.
pub(super) struct SnapshotCache {
    entries: Mutex<Entries>,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
            max_entries: MAX_ENTRIES,
            max_bytes: MAX_RETAINED_BYTES,
        }
    }
}

impl SnapshotCache {
    pub(super) async fn snapshot(
        &self,
        path: &Path,
        diagnostics: &mut Vec<Value>,
        maximum: u64,
    ) -> Option<Arc<Value>> {
        match self.read(path, maximum).await {
            Ok(value) => Some(value),
            Err(error) => {
                self.entries.lock().await.remove(path);
                if !error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                {
                    diagnostics.push(
                        json!({"file":path.file_name().and_then(|name| name.to_str()),
                        "error":error.to_string()}),
                    );
                }
                None
            }
        }
    }

    async fn read(&self, path: &Path, maximum: u64) -> Result<Arc<Value>> {
        let mut entries = self.entries.lock().await;
        let stamp = Stamp::read(path).await?;
        ensure!(
            stamp.length <= maximum,
            "Snapshot exceeds the metadata limit"
        );
        entries.clock = entries.clock.wrapping_add(1);
        let used = entries.clock;
        if let Some(entry) = entries.values.get_mut(path) {
            if entry.stamp == stamp
                && entry.read_at.elapsed() < REVALIDATE_AFTER
                && (!entry.sqlite || entry.value["finished_at"].is_i64())
            {
                ensure!(
                    entry.encoded_bytes <= maximum,
                    "Snapshot exceeds the metadata limit"
                );
                entry.used = used;
                return Ok(entry.value.clone());
            }
        }
        entries.remove(path);
        let owned = path.to_owned();
        let linked = tokio::task::spawn_blocking(move || {
            crate::metadata_store::linked_snapshot(&owned, maximum)
        })
        .await??;
        let sqlite = linked.is_some();
        let (value, encoded_bytes) = match linked {
            Some(bytes) => (serde_json::from_slice(&bytes)?, bytes.len() as u64),
            None => (read_object(path, maximum).await?, stamp.length),
        };
        let value = Arc::new(value);
        // An atomic replacement during the read is a valid old snapshot, but
        // must not be remembered as the new file. A later poll reads it again.
        if Stamp::read(path).await? != stamp {
            return Ok(value);
        }
        let weight = retained_size(&value);
        if weight <= self.max_bytes && self.max_entries > 0 {
            while entries.values.len() >= self.max_entries
                || entries.retained_bytes.saturating_add(weight) > self.max_bytes
            {
                let oldest = entries
                    .values
                    .iter()
                    .min_by_key(|(_, item)| item.used)
                    .map(|(path, _)| path.clone());
                let Some(oldest) = oldest else { break };
                entries.remove(&oldest);
            }
            entries.retained_bytes += weight;
            entries.values.insert(
                path.to_owned(),
                Entry {
                    stamp,
                    value: value.clone(),
                    weight,
                    used,
                    read_at: Instant::now(),
                    sqlite,
                    encoded_bytes,
                },
            );
        }
        Ok(value)
    }
}

impl Entries {
    fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.values.remove(path) {
            self.retained_bytes -= entry.weight;
        }
    }
}

/// Account for owned JSON strings/containers, including a conservative per-key
/// tree-node allowance. This is a cache budget, not an allocator/RSS measurement.
fn retained_size(value: &Value) -> usize {
    let children = match value {
        Value::String(text) => text.capacity(),
        Value::Array(values) => {
            values.iter().map(retained_size).sum::<usize>()
                + (values.capacity() - values.len()) * std::mem::size_of::<Value>()
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                key.capacity() + std::mem::size_of::<String>() + 64 + retained_size(value)
            })
            .sum(),
        _ => 0,
    };
    std::mem::size_of::<Value>() + children
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn sqlite_references_refresh_live_values_and_cache_finished_snapshots() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("sqlite-display-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let database = root.join("telemetry.sqlite");
        let store = crate::metadata_store::Store::open(&database)?;
        let path = root.join("mirror.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({"_metadata_store":database}))?,
        )?;
        store.put("snapshot.json", br#"{"count":1,"finished_at":null}"#)?;
        let cache = SnapshotCache::default();
        assert_eq!(cache.read(&path, 8192).await?["count"], 1);
        store.put("snapshot.json", br#"{"count":2,"finished_at":null}"#)?;
        assert_eq!(cache.read(&path, 8192).await?["count"], 2);
        store.put(
            "snapshot.json",
            &serde_json::to_vec(&json!({"count":3,"finished_at":1,"note":"x".repeat(2000)}))?,
        )?;
        let terminal = cache.read(&path, 8192).await?;
        assert!(Arc::ptr_eq(&terminal, &cache.read(&path, 8192).await?));
        assert!(cache.read(&path, 1024).await.is_err());
        drop(store);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn unchanged_snapshots_share_parsed_bytes_and_changes_and_errors_invalidate() -> Result<()>
    {
        let root = std::env::temp_dir().join(format!("display-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let path = root.join("state.json");
        fs::write(&path, br#"{"status":"opening"}"#)?;
        let cache = SnapshotCache::default();
        let first = cache.read(&path, 1024).await?;
        let second = cache.read(&path, 1024).await?;
        assert!(Arc::ptr_eq(&first, &second));
        fs::write(&path, br#"{"status":"completed","count":2}"#)?;
        let completed = cache.read(&path, 1024).await?;
        assert_eq!(completed["status"], "completed");
        assert!(!Arc::ptr_eq(&first, &completed));
        assert!(
            cache.read(&path, 4).await.is_err(),
            "A cache hit cannot bypass a smaller read limit"
        );
        fs::write(&path, b"{")?;
        let mut diagnostics = Vec::new();
        assert!(cache
            .snapshot(&path, &mut diagnostics, 1024)
            .await
            .is_none());
        assert_eq!(diagnostics.len(), 1);
        fs::remove_file(&path)?;
        assert!(cache
            .snapshot(&path, &mut diagnostics, 1024)
            .await
            .is_none());
        assert_eq!(
            diagnostics.len(),
            1,
            "Missing snapshots retain normal omission semantics"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn least_recent_entries_are_evicted_and_oversized_values_are_not_retained() -> Result<()>
    {
        let root =
            std::env::temp_dir().join(format!("bounded-display-cache-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let cache = SnapshotCache {
            max_entries: 2,
            max_bytes: 2048,
            ..Default::default()
        };
        for name in ["first", "second", "third"] {
            fs::write(root.join(name), br#"{"value":1}"#)?;
        }
        let first = cache.read(&root.join("first"), 4096).await?;
        let second = cache.read(&root.join("second"), 4096).await?;
        cache.read(&root.join("first"), 4096).await?;
        cache.read(&root.join("third"), 4096).await?;
        assert!(Arc::ptr_eq(
            &first,
            &cache.read(&root.join("first"), 4096).await?
        ));
        assert!(!Arc::ptr_eq(
            &second,
            &cache.read(&root.join("second"), 4096).await?
        ));
        fs::write(
            root.join("large"),
            json!({"value":"x".repeat(3000)}).to_string(),
        )?;
        cache.read(&root.join("large"), 4096).await?;
        let entries = cache.entries.lock().await;
        assert!(entries.values.len() <= 2);
        assert!(entries.retained_bytes <= 2048);
        assert!(!entries.values.contains_key(&root.join("large")));
        drop(entries);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
