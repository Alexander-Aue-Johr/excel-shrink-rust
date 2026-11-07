//! Page inspection ranges without copying the full plan into live status responses.
use super::*;
use std::io::{BufRead, BufReader};

pub(super) async fn list(
    State(room): State<Arc<Room>>,
    RoutePath(id): RoutePath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> HttpResult<Json<Value>> {
    safe_id(&id)?;
    let offset = query
        .get("offset")
        .map(|text| text.parse::<usize>())
        .transpose()
        .map_err(|_| ApiError::bad_request("Invalid region offset."))?
        .unwrap_or(0);
    let mut diagnostics = Vec::new();
    let snapshot = read_snapshot(
        &room
            .root
            .join(format!("groups/{id}/targeted-comparison.json")),
        &mut diagnostics,
        SNAPSHOT_LIMIT,
    )
    .await;
    if let Some(directory) = snapshot
        .as_ref()
        .and_then(|value| value["report_directory"].as_str())
    {
        let directory = PathBuf::from(directory);
        if directory.join("capture-regions.jsonl").is_file() {
            let study = room
                .root
                .parent()
                .context("Missing study directory")?
                .to_owned();
            let page = tokio::task::spawn_blocking(move || read_page(&study, &directory, offset))
                .await
                .context("Read capture plan")??;
            return Ok(Json(page));
        }
    }
    if let Some(snapshot) = snapshot.as_ref() {
        let stage = if snapshot["finished_at"].is_null() {
            Some("candidate_selection_in_progress")
        } else if !matches!(
            snapshot["status"].as_str(),
            Some("completed" | "completed_with_limitations")
        ) {
            Some("attempt_has_no_retained_region_index")
        } else {
            None
        };
        if let Some(stage) = stage {
            return Ok(Json(
                json!({"stage":stage,"total":0,"offset":0,"regions":[]}),
            ));
        }
    }
    // Older completed attempts retain their compared ranges in the portable report.
    let Json(report) = targeted::report(State(room), RoutePath(id)).await?;
    let mut regions = std::collections::BTreeSet::new();
    for pair in report["comparisons"].as_array().into_iter().flatten() {
        for finding in pair["findings"].as_array().into_iter().flatten() {
            if let (Some(sheet), Some(range)) =
                (finding["sheet"].as_str(), finding["cell"].as_str())
            {
                regions.insert((sheet.to_owned(), range.to_owned()));
            }
        }
    }
    let rows: Vec<_> = regions
        .iter()
        .skip(offset)
        .take(100)
        .map(|(sheet, range)| json!({"sheet":sheet,"range":range}))
        .collect();
    Ok(Json(
        json!({"stage":"retained_compared_regions","total":regions.len(),"offset":offset,"regions":rows}),
    ))
}

fn read_page(study: &Path, directory: &Path, offset: usize) -> Result<Value> {
    let evidence = std::fs::canonicalize(study.join("evidence"))?;
    let directory = std::fs::canonicalize(directory)?;
    ensure!(
        directory.starts_with(&evidence),
        "Capture plan is outside retained evidence."
    );
    let file = std::fs::File::open(directory.join("capture-regions.jsonl"))?;
    let mut lines = BufReader::new(file).lines();
    let mut header: Value =
        serde_json::from_str(&lines.next().context("Missing capture plan header")??)?;
    let rows = lines
        .skip(offset)
        .take(100)
        .map(|line| Ok(serde_json::from_str::<Value>(&line?)?))
        .collect::<Result<Vec<_>>>()?;
    header["offset"] = json!(offset);
    header["regions"] = json!(rows);
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn pages_are_bounded_and_cannot_escape_evidence() -> Result<()> {
        let root = std::env::temp_dir().join(format!("capture-page-{}", uuid::Uuid::new_v4()));
        let directory = root.join("evidence/attempt");
        std::fs::create_dir_all(&directory)?;
        let mut file = std::fs::File::create(directory.join("capture-regions.jsonl"))?;
        writeln!(
            file,
            "{}",
            json!({"total":205,"stage":"measured_capture_regions"})
        )?;
        for row in 1..=205 {
            writeln!(
                file,
                "{}",
                json!({"sheet":"Sheet1","range":format!("A{row}")})
            )?;
        }
        drop(file);
        let first = read_page(&root, &directory, 0)?;
        assert_eq!(first["regions"].as_array().unwrap().len(), 100);
        let last = read_page(&root, &directory, 200)?;
        assert_eq!(last["regions"].as_array().unwrap().len(), 5);
        assert_eq!(last["regions"][0]["range"], "A201");
        assert!(read_page(&root, &root, 0).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
