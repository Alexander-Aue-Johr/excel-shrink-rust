//! Preserve independent run observations in a verified, navigable report collection.
use super::{evidence, export_site, read_json_file, write_site};
use anyhow::{ensure, Context, Result};
use deltasieve_engine::reuse::{file_sha256, PairFingerprint};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

struct Source {
    root: PathBuf,
    digest: String,
    snapshot_id: String,
    run: Value,
}

/// Publish into a new directory. Failures retain an incomplete marker and copied evidence.
pub(crate) fn merge_reports(inputs: &[PathBuf], output: &Path) -> Result<PathBuf> {
    ensure!(inputs.len() >= 2, "Select at least two reports to merge");
    let destination = crate::validation::storage::destination(output)?;
    ensure!(
        !destination.try_exists()?,
        "Merge destination already exists"
    );
    let sources = read_sources(inputs, &destination)?;
    let destination = evidence::create_destination(&sources[0].root, &destination)?;
    fs::write(
        destination.join(".incomplete"),
        b"Report merge in progress\n",
    )?;
    fs::create_dir(destination.join("runs"))?;
    let mut observations = Vec::new();
    let mut provenance = Vec::new();
    let mut copied = BTreeMap::new();
    let mut equivalent: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for source in sources {
        let prefix = format!("runs/{}", source.snapshot_id);
        provenance.push(
            json!({"source_path":source.root,"run_id":source.run["run_id"],
            "source_run_sha256":source.digest,"snapshot_id":source.snapshot_id,"snapshot":format!("{prefix}/run.json"),
            "source_metadata":format!("{prefix}/source-run.json"),
            "validation_signature":source.run["validation_signature"],"config":source.run["config"],
            "selection":source.run["selection"],"preparation":source.run["preparation"],
            "status":source.run["status"],"started_at":source.run["started_at"],
            "finished_at":source.run["finished_at"],"previous_merge":source.run["merge"]}),
        );
        if copied.insert(source.snapshot_id.clone(), ()).is_some() {
            ensure!(
                file_sha256(&source.root.join("run.json"))? == source.digest,
                "Repeated source metadata changed during merge"
            );
            continue;
        }
        export_site(&source.root, &destination.join(&prefix))?;
        ensure!(
            file_sha256(&source.root.join("run.json"))? == source.digest,
            "Source run metadata changed during merge"
        );
        let source_bytes = fs::read(source.root.join("run.json"))?;
        ensure!(
            format!("{:x}", Sha256::digest(&source_bytes)) == source.digest,
            "Source metadata changed before preservation"
        );
        evidence::write_new(
            &destination.join(&prefix).join("source-run.json"),
            &source_bytes,
        )?;
        let snapshot = read_json_file(&destination.join(&prefix).join("run.json"))?;
        for pair in snapshot["pairs"]
            .as_array()
            .context("Source report has no pair index")?
        {
            let mut observation = pair.clone();
            let id = pair["id"].as_str().context("Source pair has no identity")?;
            let merged_id = digest(&json!([source.snapshot_id, id]))?;
            observation["id"] = json!(merged_id);
            observation["display_name"] = json!(format!(
                "{} · run {}",
                pair["display_name"]
                    .as_str()
                    .or(pair["name"].as_str())
                    .unwrap_or(id),
                &source.digest[..8]
            ));
            for field in ["report", "identity"] {
                if let Some(relative) = pair[field].as_str() {
                    evidence::relative_path(relative)?;
                    observation[field] = json!(format!("{prefix}/{relative}"));
                }
            }
            observation["merge_source"] = json!({"source_run_sha256":source.digest,"snapshot_id":source.snapshot_id,
                "pair_id":id,"previous":pair["merge_source"]});
            if let Some(key) = comparable_key(
                &destination,
                &observation,
                &source.run["validation_signature"],
            )? {
                equivalent
                    .entry(key)
                    .or_default()
                    .push(json!({"id":merged_id,
                    "status":pair["status"],"different_regions":pair["different_regions"],
                    "source_run_sha256":source.digest}));
            }
            observations.push(observation);
        }
    }
    let overlaps: Vec<Value> = equivalent
        .into_iter()
        .filter(|(_, rows)| rows.len() > 1)
        .map(|(key, rows)| {
            let first = json!([rows[0]["status"], rows[0]["different_regions"]]);
            let conflict = rows
                .iter()
                .skip(1)
                .any(|row| json!([row["status"], row["different_regions"]]) != first);
            json!({"key":key,"potential_outcome_conflict":conflict,"observations":rows})
        })
        .collect();
    let conflicts = overlaps
        .iter()
        .filter(|row| row["potential_outcome_conflict"] == true)
        .count();
    let unresolved = observations
        .iter()
        .filter(|row| {
            !matches!(
                row["status"].as_str(),
                Some("completed_with_limitations" | "reused")
            )
        })
        .count();
    let merge = json!({
        "schema": 1,
        "sources": provenance,
        "distinct_run_snapshots": copied.len(),
        "observations": observations.len(),
        "unresolved_observations": unresolved,
        "potential_conflicts": conflicts,
        "overlaps": overlaps,
        "meaning": "Counts are retained observations, not unique corpus coverage. Matching summaries do not prove equivalent findings. All observations remain separate.",
        "comparability": "Overlap checks require complete package fingerprints and the same nonempty declared environment and validation signature. Unclassified observations remain independent."
    });
    let status = if unresolved + conflicts == 0 {
        "completed_with_limitations"
    } else {
        "completed_with_errors"
    };
    let run = json!({
        "schema": 1,
        "tool": "DeltaSieve",
        "version": env!("CARGO_PKG_VERSION"),
        "run_id": uuid::Uuid::new_v4().to_string(),
        "status": status,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "finished_at": chrono::Utc::now().to_rfc3339(),
        "index_complete": true,
        "scope": "Collection of independent recorded observations; see each source run for coverage and environment.",
        "summary": {
            "total": observations.len(),
            "processed_pairs": observations.len() - unresolved,
            "failed_pairs": unresolved
        },
        "merge": merge,
        "pairs": observations,
        "replay": {
            "available": false,
            "reason": "Source run replays retain their separate clocks in the embedded source reports."
        }
    });
    publish_collection(&destination, &run)
}

fn publish_collection(destination: &Path, run: &Value) -> Result<PathBuf> {
    evidence::write_new(
        &destination.join("run.json"),
        &serde_json::to_vec_pretty(&run)?,
    )?;
    let mut audit_files = vec!["run.json".to_owned()];
    for source in run["merge"]["sources"]
        .as_array()
        .context("Collection has no sources")?
    {
        for field in ["snapshot", "source_metadata"] {
            audit_files.push(
                source[field]
                    .as_str()
                    .context("Source metadata path missing")?
                    .to_owned(),
            );
        }
    }
    audit_files.sort();
    audit_files.dedup();
    let review = super::publication::audit(destination, &audit_files)?;
    evidence::write_new(
        &destination.join("publication-review.json"),
        &serde_json::to_vec_pretty(&review)?,
    )?;
    let index = write_site(&destination, &run)?;
    fs::remove_file(destination.join(".incomplete"))?;
    Ok(index)
}

/// Export each source below its existing namespace, preserving source replay links.
pub(super) fn export_collection(source: &Path, run: &Value, output: &Path) -> Result<PathBuf> {
    let destination = evidence::create_destination(source, output)?;
    super::atlas::copy_labels(source, &destination)?;
    fs::write(
        destination.join(".incomplete"),
        b"Report collection export in progress\n",
    )?;
    fs::create_dir(destination.join("runs"))?;
    let mut copied = BTreeSet::new();
    for entry in run["merge"]["sources"]
        .as_array()
        .context("Collection has no sources")?
    {
        let hash = entry["snapshot_id"]
            .as_str()
            .context("Source snapshot has no checksum")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "Invalid source snapshot checksum"
        );
        if !copied.insert(hash) {
            continue;
        }
        let relative = format!("runs/{hash}/run.json");
        ensure!(
            entry["snapshot"] == relative,
            "Invalid source snapshot location"
        );
        let snapshot = evidence::confined_file(source, &relative)?;
        export_site(
            snapshot
                .parent()
                .context("Source snapshot has no directory")?,
            &destination.join(format!("runs/{hash}")),
        )?;
        let metadata_relative = format!("runs/{hash}/source-run.json");
        ensure!(
            entry["source_metadata"] == metadata_relative,
            "Invalid original metadata location"
        );
        let metadata = evidence::confined_file(source, &metadata_relative)?;
        let bytes = fs::read(metadata)?;
        ensure!(
            entry["source_run_sha256"] == format!("{:x}", Sha256::digest(&bytes)),
            "Original source metadata checksum changed"
        );
        evidence::write_new(&destination.join(metadata_relative), &bytes)?;
    }
    publish_collection(&destination, run)
}

fn digest(value: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

/// Partial component selections cannot establish a complete workbook overlap.
fn comparable_key(root: &Path, pair: &Value, signature: &Value) -> Result<Option<String>> {
    if signature["environment_key"]
        .as_str()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Ok(None);
    }
    let Some(relative) = pair["identity"].as_str() else {
        return Ok(None);
    };
    let identity: PairFingerprint =
        serde_json::from_slice(&fs::read(evidence::confined_file(root, relative)?)?)?;
    let selection = &identity.selection;
    if identity.schema != 1
        || ![
            selection.worksheets,
            selection.workbook,
            selection.styles,
            selection.shared_strings,
            selection.relationships,
            selection.drawings,
            selection.charts,
            selection.other,
        ]
        .into_iter()
        .all(|value| value)
    {
        return Ok(None);
    }
    Ok(Some(digest(
        &json!({"name":pair["name"],"source_id":pair["preparation"]["source_id"],
        "original":identity.original,"candidate":identity.candidate,"signature":signature}),
    )?))
}

/// Resolve and validate every source before creating output directories.
fn read_sources(inputs: &[PathBuf], destination: &Path) -> Result<Vec<Source>> {
    let mut sources = Vec::new();
    for input in inputs {
        let root = crate::validation::report_root(input)?;
        ensure!(
            !root.join(".incomplete").try_exists()?,
            "Source merge is incomplete"
        );
        ensure!(
            !destination.starts_with(&root) && !root.starts_with(&destination),
            "Merged output must be separate from every source report"
        );
        let metadata_hash = file_sha256(&root.join("run.json"))?;
        let run = crate::validation::read_run(&root)?;
        ensure!(
            file_sha256(&root.join("run.json"))? == metadata_hash,
            "Source metadata changed during inspection"
        );
        ensure!(
            run["schema"] == 1 && run["tool"] == "DeltaSieve",
            "Unsupported source report"
        );
        ensure!(
            matches!(
                run["status"].as_str(),
                Some("completed_with_limitations" | "completed_with_errors" | "failed")
            ),
            "Finish the source run before merging its report"
        );
        sources.push(Source {
            snapshot_id: digest(&json!([metadata_hash, root]))?,
            digest: metadata_hash,
            root,
            run,
        });
    }
    sources.sort_by(|a, b| a.digest.cmp(&b.digest).then(a.root.cmp(&b.root)));
    Ok(sources)
}
