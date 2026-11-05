//! Read-only formula views. Legacy source examples never pretend to be the
//! actual rebuilt values captured by a historical comparison.
use super::*;
use crate::regression::formula_trace;
use sha2::{Digest, Sha256};

static INSPECTIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(serde::Deserialize)]
pub(super) struct Options {
    region: String,
}

pub(super) async fn inspect(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    Query(options): Query<Options>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let Json(report) = targeted::report(State(room.clone()), RoutePath(id.clone())).await?;
    let (pair, kind, finding) = selection(&options.region)?;
    let entry = report["comparisons"]
        .get(pair)
        .and_then(|pair| pair[kind].get(finding))
        .ok_or_else(|| ApiError::not_found("Unknown comparison region"))?;
    if let Some(retained) = report["formula_diagnostics"]["regions"]
        .as_array()
        .and_then(|regions| {
            regions.iter().find(|region| {
                region["pair_index"] == pair
                    && region["kind"] == kind
                    && region["finding_index"] == finding
            })
        })
    {
        return Ok(Json(
            json!({"report_id":report["_evidence_id"],"diagnosis":retained["diagnosis"]}),
        ));
    }
    if kind != "findings" {
        return Ok(Json(
            json!({"report_id":report["_evidence_id"],"diagnosis":{"evidence":"unavailable","reason":"Chart-series formula tracing is not available for this finding."}}),
        ));
    }
    let sheet = entry["sheet"]
        .as_str()
        .context("Region sheet missing")?
        .to_owned();
    let range = entry["cell"]
        .as_str()
        .context("Region address missing")?
        .to_owned();
    let record = read_object(
        &room
            .root
            .parent()
            .context("Study directory missing")?
            .join(format!("records/{id}.json")),
        SNAPSHOT_LIMIT,
    )
    .await?;
    let identity = json!({"report":report["_evidence_id"],"region":options.region,"source":record["baseline_sha256"],"producer":record["prepared_by"]});
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?));
    let cache = room
        .root
        .parent()
        .unwrap()
        .join(".diagnostics/formula-v2")
        .join(format!("{hash}.json"));
    if cache.is_file() {
        return Ok(Json(read_object(&cache, SNAPSHOT_LIMIT).await?));
    }
    let _permit = INSPECTIONS
        .acquire()
        .await
        .context("Formula inspections stopped")?;
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let directory = std::env::temp_dir()
            .join("excel-shrink-formula-inspection")
            .join(uuid::Uuid::new_v4().to_string());
        let source = inspection::copy_workbook(&room.root, &id, "original", &directory)?;
        let diagnosis = formula_trace::read(&source)
            .and_then(|book| formula_trace::source_examples(&book, &sheet, &range));
        // Remove only the exact owned copy and its now-empty directory.
        if let Err(error) =
            std::fs::remove_file(&source).and_then(|_| std::fs::remove_dir(&directory))
        {
            tracing::warn!(path=%source.display(),%error,"Cannot remove formula inspection copy");
        }
        let diagnosis = diagnosis.unwrap_or_else(
            |error| json!({"evidence":"unavailable","reason":format!("{error:#}")}),
        );
        let result = json!({"report_id":report["_evidence_id"],"diagnosis":diagnosis});
        let bytes = serde_json::to_vec(&result)?;
        anyhow::ensure!(
            bytes.len() <= 1024 * 1024,
            "Formula explanation exceeds the display budget"
        );
        std::fs::create_dir_all(cache.parent().context("Missing formula cache parent")?)?;
        crate::regression::write_atomic_artifact(&cache, &bytes)?;
        Ok(result)
    })
    .await
    .context("Formula inspection task failed")??;
    Ok(Json(result))
}

fn selection(value: &str) -> Result<(usize, &str, usize)> {
    let parts: Vec<_> = value.split(':').collect();
    anyhow::ensure!(
        parts.len() == 3 && matches!(parts[1], "findings" | "charts"),
        "Invalid formula region selector"
    );
    Ok((parts[0].parse()?, parts[1], parts[2].parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selectors_only_address_a_report_pair_and_finding() {
        assert_eq!(selection("2:findings:3").unwrap(), (2, "findings", 3));
        for invalid in ["../file", "0:unknown:1", "0:findings:-1", "0:findings:0:1"] {
            assert!(selection(invalid).is_err());
        }
    }
}
