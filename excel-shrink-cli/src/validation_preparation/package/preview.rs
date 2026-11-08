use super::*;
use crate::validation_preparation::{self, Input};

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Complete,
    Missing,
    Unsuccessful,
    BaselineMismatch,
    SourceMismatch,
    LocalBaselineUnavailable,
    LocalBaselineChanged,
    AmbiguousGeneration,
}

#[derive(Serialize)]
pub(super) struct Item {
    source_id: String,
    name: String,
    pub status: Status,
    detail: Option<String>,
    pub selected_generation: Option<String>,
    candidates: Vec<Value>,
}

#[derive(Serialize)]
pub(crate) struct Preview {
    version: u32,
    selection_sha256: String,
    selected: usize,
    counts: BTreeMap<String, usize>,
    offset: usize,
    limit: usize,
    items: Vec<Item>,
}

/// Reconcile the entire selection but return only one bounded page of details.
/// A compatible generation requires the exact source occurrence and baseline bytes.
pub(crate) fn preview(
    package: &Path,
    manifest: &Path,
    offset: usize,
    limit: usize,
) -> Result<Preview> {
    ensure!(
        (1..=1000).contains(&limit),
        "Preview limit must be between 1 and 1000"
    );
    let header = verify(package)?;
    ensure!(
        header.identity_scope == "source-manifest",
        "Corpus reuse requires a source-manifest package"
    );
    let inputs = validation_preparation::read(manifest)?;
    let selection_sha256 = validation_preparation::selection_hash(&inputs)?;
    let root = fs::canonicalize(package)?;
    let connection = Connection::open_with_flags(
        confined_file(&root, "metadata.sqlite")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    connection.pragma_update(None, "trusted_schema", false)?;
    let mut statement = connection.prepare("SELECT generation_id, status, provenance_id, payload FROM entries WHERE source_id=?1 ORDER BY generation_id")?;
    let mut counts = BTreeMap::new();
    let mut items = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let item = inspect(&mut statement, input)?;
        let key = serde_json::to_value(&item.status)?
            .as_str()
            .context("Status must be text")?
            .to_owned();
        *counts.entry(key).or_insert(0) += 1;
        if index >= offset && index - offset < limit {
            items.push(item);
        }
    }
    Ok(Preview {
        version: 1,
        selection_sha256,
        selected: inputs.len(),
        counts,
        offset,
        limit,
        items,
    })
}

pub(super) fn inspect(statement: &mut rusqlite::Statement<'_>, input: &Input) -> Result<Item> {
    let mut rows = statement.query([&input.source_id])?;
    let mut candidates = Vec::new();
    while let Some(row) = rows.next()? {
        let payload: Value = serde_json::from_str(&row.get::<_, String>(3)?)?;
        candidates.push(json!({
            "generation_id":row.get::<_, String>(0)?,
            "status":row.get::<_, String>(1)?,
            "provenance_id":row.get::<_, Option<String>>(2)?,
            "baseline_sha256":payload["source_sha256"],
            "raw_source":payload["raw_source"],
            "error":payload["error"]
        }));
    }
    let (status, detail, selected_generation) = classify(input, &candidates)?;
    Ok(Item {
        source_id: input.source_id.clone(),
        name: input.name.clone(),
        status,
        detail,
        selected_generation,
        candidates,
    })
}

fn classify(
    input: &Input,
    candidates: &[Value],
) -> Result<(Status, Option<String>, Option<String>)> {
    if candidates.is_empty() {
        return Ok((Status::Missing, None, None));
    }
    let prepared: Vec<_> = candidates
        .iter()
        .filter(|entry| entry["status"] == "prepared")
        .collect();
    if prepared.is_empty() {
        return Ok((Status::Unsuccessful, None, None));
    }
    let raw = input
        .raw_source
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    let sources: Vec<_> = prepared
        .into_iter()
        .filter(|entry| raw.as_ref().is_none_or(|raw| entry["raw_source"] == *raw))
        .collect();
    if sources.is_empty() {
        return Ok((Status::SourceMismatch, None, None));
    }
    let expected = input
        .expected_sha256
        .as_deref()
        .context("Selection baseline hash missing")?;
    let matching = sources
        .iter()
        .filter(|entry| entry["baseline_sha256"] == expected)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok((Status::BaselineMismatch, None, None));
    }
    match artifact_cache::hash_file(&input.baseline) {
        Ok(actual) if actual != expected => {
            return Ok((
                Status::LocalBaselineChanged,
                Some("Local baseline differs from the selected manifest".into()),
                None,
            ))
        }
        Err(error) => {
            return Ok((
                Status::LocalBaselineUnavailable,
                Some(format!("{error:#}")),
                None,
            ))
        }
        _ => {}
    }
    if matching.len() > 1 {
        return Ok((Status::AmbiguousGeneration, None, None));
    }
    let generation = matching[0]["generation_id"]
        .as_str()
        .context("Generation missing")?
        .to_owned();
    Ok((Status::Complete, None, Some(generation)))
}
