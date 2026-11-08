//! Bind portable preparation identities and producer measurements to ordinary comparisons.
use super::*;
use crate::regression::artifact_cache;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) struct ImportedEvidence {
    pub summary: Value,
    entries: BTreeMap<String, Value>,
}

impl ImportedEvidence {
    /// Read a completed import index and retain only identities that bind to its receipts.
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path)?;
        let index: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            index["version"] == 1 && index["operation"] == "import-check-performance",
            "Unsupported preparation import index"
        );
        ensure!(
            !path
                .parent()
                .context("Import index has no parent")?
                .join("incomplete.json")
                .exists(),
            "Preparation import is incomplete"
        );
        let provenance = index["provenance"]
            .as_object()
            .context("Import provenance missing")?;
        for (id, producer) in provenance {
            ensure!(
                artifact_cache::sha256(&serde_json::to_vec(producer)?) == *id,
                "Import provenance identity mismatch"
            );
        }
        let mut entries = BTreeMap::new();
        for entry in index["entries"]
            .as_array()
            .context("Import entries missing")?
        {
            let source_id = entry["source_id"]
                .as_str()
                .context("Imported source ID missing")?;
            let name = entry["pair_file"]
                .as_str()
                .context("Imported pair filename missing")?;
            ensure!(
                name == format!("{}.xlsx", artifact_cache::sha256(source_id.as_bytes())),
                "Imported pair filename does not bind to its source ID"
            );
            let producer = entry["provenance_id"]
                .as_str()
                .context("Imported producer missing")?;
            ensure!(
                provenance.contains_key(producer),
                "Imported producer reference missing"
            );
            let payload = &entry["prepared_evidence"];
            ensure!(
                payload["source_id"] == source_id && payload["status"] == "prepared",
                "Imported source receipt mismatch"
            );
            let generation =
                artifact_cache::sha256(&serde_json::to_vec(&(payload, &Some(producer)))?);
            ensure!(
                entry["generation_id"] == generation,
                "Imported generation receipt mismatch"
            );
            ensure!(
                entries.insert(name.to_owned(), entry.clone()).is_none(),
                "Duplicate imported source occurrence"
            );
        }
        Ok(Self {
            summary: json!({"import_index_sha256":artifact_cache::sha256(&bytes),
            "package_database_sha256":index["package_database_sha256"],"selection_sha256":index["selection_sha256"],
            "provenance":provenance,"import_options":index["import_options"],"imported_at":index["imported_at"]}),
            entries,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn record(&self, name: &str) -> Result<Value> {
        let entry = self
            .entries
            .get(name)
            .context("Workbook has no imported identity")?;
        let receipt = &entry["prepared_evidence"];
        Ok(
            json!({"source_id":entry["source_id"],"display_name":entry["name"],
            "generation_id":entry["generation_id"],"provenance_id":entry["provenance_id"],
            "requested_baseline_sha256":entry["requested_baseline_sha256"],
            "raw_source":receipt["raw_source"],"states":receipt["states"],
            "producer_measurements":receipt["measurements"],"native_result":receipt["native_result"]}),
        )
    }

    /// Verify actual comparison inputs against their imported states before rendering or reuse.
    pub(crate) fn verify_pair(&self, name: &str, original: &Path, candidate: &Path) -> Result<()> {
        let entry = self
            .entries
            .get(name)
            .context("Workbook has no imported identity")?;
        for (path, role) in [(original, "baseline"), (candidate, "after-optimization")] {
            let state = &entry["prepared_evidence"]["states"][role];
            ensure!(
                state["bytes"].as_u64() == Some(fs::metadata(path)?.len()),
                "Imported {role} length changed"
            );
            ensure!(
                state["sha256"] == artifact_cache::hash_file(path)?,
                "Imported {role} bytes changed"
            );
        }
        Ok(())
    }
}
