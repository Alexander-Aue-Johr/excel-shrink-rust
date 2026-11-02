//! Reuse immutable workbook analyses by complete archive content, not mutable filenames.
use super::xml_changes::Snapshot;
use crate::regression::artifact_cache;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Default, Serialize)]
pub(super) struct Statistics {
    pub reads: usize,
    pub parsed_versions: usize,
    pub reused_versions: usize,
    pub elapsed_ms: u64,
}

pub(super) struct Snapshots {
    versions: BTreeMap<String, Snapshot>,
    scratch: PathBuf,
    statistics: Statistics,
}

impl Snapshots {
    pub fn new(scratch: PathBuf) -> Self {
        Self {
            versions: BTreeMap::new(),
            scratch,
            statistics: Statistics::default(),
        }
    }

    /// Hash on every read so save/rebuild of the same path cannot reuse stale cell values.
    /// Snapshot clones share owned cell streams and immutable layout metadata.
    pub fn read(&mut self, path: &Path) -> Result<Snapshot> {
        let started = Instant::now();
        self.statistics.reads += 1;
        let digest = artifact_cache::hash_file(path)?;
        let snapshot = if let Some(snapshot) = self.versions.get(&digest) {
            self.statistics.reused_versions += 1;
            snapshot.clone()
        } else {
            let snapshot = Snapshot::read_with_scratch(path, &self.scratch)?;
            ensure!(
                artifact_cache::hash_file(path)? == digest,
                "Workbook changed during XML analysis"
            );
            self.statistics.parsed_versions += 1;
            self.versions.insert(digest, snapshot.clone());
            snapshot
        };
        self.statistics.elapsed_ms += started.elapsed().as_millis() as u64;
        Ok(snapshot)
    }

    pub fn statistics(&self) -> Statistics {
        self.statistics.clone()
    }
}
