use super::*;
use deltasieve_engine::reuse::PartSelection;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("report-merge-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    fn source(&self, name: &str, different: bool, environment: &str) -> Result<PathBuf> {
        let root = self.0.join(name);
        fs::create_dir_all(root.join("raw"))?;
        fs::create_dir(root.join("identities"))?;
        let raw = json!({"status":"completed_with_limitations",
            "transitions":[{"before_file":"C:/private/source.xlsx"}],"comparisons":[{
            "left":"OR","right":"SDR","findings":if different {
                vec![json!({"presence_differs":true,"sheet":"Sheet1","range":"A1"})]
            } else {Vec::new()}}]});
        fs::write(root.join("raw/report.json"), serde_json::to_vec(&raw)?)?;
        super::super::package_comparison(&root.join("raw"), &root.join("pairs/pair"))?;
        let identity = json!({"schema":1,"selection":PartSelection::default(),
            "original":{"xl/worksheets/sheet1.xml":{"bytes":1,"sha256":"original"}},
            "candidate":{"xl/worksheets/sheet1.xml":{"bytes":1,"sha256":"candidate"}}});
        fs::write(
            root.join("identities/pair.json"),
            serde_json::to_vec(&identity)?,
        )?;
        let run = json!({"schema":1,"tool":"DeltaSieve","run_id":name,"index_complete":true,
            "status":"completed_with_limitations","finished_at":"2026-11-08T00:00:00Z",
            "validation_signature":{"environment_key":environment,"capture_policy":"fixture"},
            "config":{"partition":{"count":2,"index":1}},
            "pairs":[{"id":"pair","name":"book.xlsx","status":"completed_with_limitations",
                "report":"pairs/pair/report.json","different_regions":usize::from(different),
                "identity":"identities/pair.json","identity_sha256":file_sha256(&root.join("identities/pair.json"))?}]});
        fs::write(root.join("run.json"), serde_json::to_vec(&run)?)?;
        Ok(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn preserves_observations_and_source_bytes_and_deduplicates_identical_snapshots() -> Result<()> {
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "machine-environment")?;
    let b = fixture.source("b", false, "machine-environment")?;
    let a_hash = file_sha256(&a.join("run.json"))?;
    let b_hash = file_sha256(&b.join("run.json"))?;
    let output = fixture.0.join("merged");
    assert!(merge_reports(&[a.clone(), a.clone(), b.clone()], &output)?.is_file());
    let run = read_json_file(&output.join("run.json"))?;
    assert_eq!(run["pairs"].as_array().unwrap().len(), 2);
    assert_eq!(run["merge"]["sources"].as_array().unwrap().len(), 3);
    assert_eq!(run["merge"]["distinct_run_snapshots"], 2);
    assert_eq!(run["merge"]["overlaps"].as_array().unwrap().len(), 1);
    assert_eq!(run["merge"]["potential_conflicts"], 0);
    assert_ne!(run["pairs"][0]["id"], run["pairs"][1]["id"]);
    for pair in run["pairs"].as_array().unwrap() {
        let report =
            evidence::confined_file(&output.canonicalize()?, pair["report"].as_str().unwrap())?;
        super::super::verify_comparison(report.parent().unwrap())?;
    }
    assert_eq!(file_sha256(&a.join("run.json"))?, a_hash);
    assert_eq!(file_sha256(&b.join("run.json"))?, b_hash);
    assert!(!output.join(".incomplete").exists());
    let review = read_json_file(&output.join("publication-review.json"))?;
    assert!(review["metadata_findings"].as_array().unwrap().iter().any(|finding| {
        finding["file"].as_str().is_some_and(|file| file.ends_with("pairs/pair/report.json"))
            && finding["pointer"] == "/transitions/0/before_file"
    }));
    assert!(!serde_json::to_string(&review)?.contains("C:/private/source.xlsx"));
    let exported = fixture.0.join("exported");
    export_site(&output, &exported)?;
    for source in run["merge"]["sources"].as_array().unwrap() {
        let metadata = exported.join(source["source_metadata"].as_str().unwrap());
        assert_eq!(
            file_sha256(&metadata)?,
            source["source_run_sha256"].as_str().unwrap()
        );
        assert!(exported
            .join(source["snapshot"].as_str().unwrap())
            .with_file_name("index.html")
            .is_file());
    }
    Ok(())
}

#[test]
fn missing_environment_and_partial_fingerprints_do_not_claim_comparability() -> Result<()> {
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "")?;
    let b = fixture.source("b", false, "")?;
    let output = fixture.0.join("merged");
    merge_reports(&[a, b], &output)?;
    let run = read_json_file(&output.join("run.json"))?;
    assert!(run["merge"]["overlaps"].as_array().unwrap().is_empty());
    let root = output.canonicalize()?;
    let pair = &run["pairs"][0];
    let path = evidence::confined_file(&root, pair["identity"].as_str().unwrap())?;
    let mut identity = read_json_file(&path)?;
    identity["selection"]["styles"] = json!(false);
    fs::write(path, serde_json::to_vec(&identity)?)?;
    assert!(comparable_key(&root, pair, &json!({"environment_key":"known"}))?.is_none());
    Ok(())
}

#[test]
fn conflicting_summaries_are_retained_and_different_environments_stay_independent() -> Result<()> {
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "environment-one")?;
    let b = fixture.source("b", true, "environment-one")?;
    let c = fixture.source("c", true, "environment-two")?;
    let output = fixture.0.join("merged");
    merge_reports(&[a, b, c], &output)?;
    let run = read_json_file(&output.join("run.json"))?;
    assert_eq!(run["merge"]["potential_conflicts"], 1);
    assert_eq!(run["status"], "completed_with_errors");
    assert_eq!(run["pairs"].as_array().unwrap().len(), 3);
    assert_eq!(
        run["merge"]["overlaps"][0]["observations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    Ok(())
}

#[test]
fn corrupt_evidence_retains_incomplete_output_and_existing_destinations_are_preserved() -> Result<()>
{
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "")?;
    let b = fixture.source("b", false, "")?;
    fs::write(b.join("pairs/pair/report.json"), b"corrupted")?;
    let output = fixture.0.join("merged");
    assert!(merge_reports(&[a.clone(), b.clone()], &output).is_err());
    assert!(output.join(".incomplete").exists());
    let sentinel = output.join("preserve.txt");
    fs::write(&sentinel, b"preserve")?;
    assert!(merge_reports(&[a, b], &output).is_err());
    assert_eq!(fs::read(sentinel)?, b"preserve");
    Ok(())
}

#[test]
fn rejects_nested_output_and_active_sources_before_creating_a_destination() -> Result<()> {
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "")?;
    let b = fixture.source("b", false, "")?;
    let nested = b.join("nested/output");
    assert!(merge_reports(&[a.clone(), b.clone()], &nested).is_err());
    assert!(!b.join("nested").exists());
    let mut run = read_json_file(&b.join("run.json"))?;
    run["status"] = json!("validating");
    fs::write(b.join("run.json"), serde_json::to_vec(&run)?)?;
    let output = fixture.0.join("merged");
    assert!(merge_reports(&[a, b], &output).is_err());
    assert!(!output.exists());
    Ok(())
}

#[test]
fn identical_metadata_in_a_different_directory_does_not_bypass_evidence_checks() -> Result<()> {
    let fixture = Fixture::new()?;
    let a = fixture.source("a", false, "known")?;
    let b = fixture.0.join("copied");
    for relative in [
        "run.json",
        "identities/pair.json",
        "pairs/pair/report.json",
        "pairs/pair/bundle.json",
    ] {
        let target = b.join(relative);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(a.join(relative), target)?;
    }
    let good = fixture.0.join("good");
    merge_reports(&[a.clone(), b.clone()], &good)?;
    let run = read_json_file(&good.join("run.json"))?;
    assert_eq!(run["pairs"].as_array().unwrap().len(), 2);
    assert_eq!(
        run["merge"]["sources"][0]["source_run_sha256"],
        run["merge"]["sources"][1]["source_run_sha256"]
    );
    assert_ne!(
        run["merge"]["sources"][0]["snapshot_id"],
        run["merge"]["sources"][1]["snapshot_id"]
    );
    fs::write(b.join("pairs/pair/report.json"), b"corrupted")?;
    assert!(merge_reports(&[a, b], &fixture.0.join("bad")).is_err());
    Ok(())
}
