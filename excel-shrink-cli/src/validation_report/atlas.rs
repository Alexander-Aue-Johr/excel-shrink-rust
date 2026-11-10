//! Derive a lazy static atlas from retained, independent comparison reports.
use super::{evidence, inventory, read_json_file, site};
use anyhow::{Context, Result};
use flate2::{write::GzEncoder, Compression};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Write},
    path::Path,
};

const ASSETS: &[(&str, &str)] = &[
    ("live.js", include_str!("../../office-capture/sheet-atlas/live.js")),
    ("live.css", include_str!("../../office-capture/sheet-atlas/live.css")),
    (
        "app.js",
        include_str!("../../office-capture/sheet-atlas/app.js"),
    ),
    (
        "model.js",
        include_str!("../../office-capture/sheet-atlas/model.js"),
    ),
    (
        "gallery.js",
        include_str!("../../office-capture/sheet-atlas/gallery.js"),
    ),
    (
        "card-painter.js",
        include_str!("../../office-capture/sheet-atlas/card-painter.js"),
    ),
    (
        "comparison.js",
        include_str!("../../office-capture/sheet-atlas/comparison.js"),
    ),
    (
        "sheet-renderer.js",
        include_str!("../../office-capture/sheet-atlas/sheet-renderer.js"),
    ),
    (
        "data.js",
        include_str!("../../office-capture/sheet-atlas/data.js"),
    ),
    (
        "style.css",
        include_str!("../../office-capture/sheet-atlas/style.css"),
    ),
    (
        "atlas.css",
        include_str!("../../office-capture/sheet-atlas/atlas.css"),
    ),
    (
        "vendor/three.module.js",
        include_str!("../../office-capture/sheet-atlas/vendor/three.module.js"),
    ),
    (
        "vendor/three.core.js",
        include_str!("../../office-capture/sheet-atlas/vendor/three.core.js"),
    ),
    (
        "vendor/THREE-LICENSE.txt",
        include_str!("../../office-capture/sheet-atlas/vendor/THREE-LICENSE.txt"),
    ),
];

pub(crate) fn live_asset(path: &str) -> Option<(&'static str, &'static str)> {
    if path == "" || path == "index.html" {
        return Some(("text/html; charset=utf-8", include_str!("../../office-capture/sheet-atlas/live.html")));
    }
    let name = path.strip_prefix("atlas-assets/")?;
    let (_, content) = ASSETS.iter().find(|(candidate, _)| *candidate == name)?;
    let mime = if name.ends_with(".js") { "text/javascript" }
        else if name.ends_with(".css") { "text/css" } else { "text/plain" };
    Some((mime, content))
}

pub(crate) fn label_atlas(root: &Path, labels: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let labels = read_json_file(labels)?;
    anyhow::ensure!(
        labels.is_object(),
        "Atlas labels must be keyed by source run ID"
    );
    for label in labels.as_object().unwrap().values() {
        anyhow::ensure!(
            label["corpus"].is_string() && label["variant"].is_string(),
            "Each atlas label needs a corpus and variant string"
        );
    }
    site::write_derived(
        &root,
        &root.join("atlas-labels.json"),
        &serde_json::to_vec_pretty(&labels)?,
    )?;
    write(&root, &crate::validation::read_run(&root)?)?;
    Ok(())
}

pub(super) fn copy_labels(source: &Path, destination: &Path) -> Result<()> {
    if source.join("atlas-cohort.json").is_file() {
        let path = evidence::confined_file(source, "atlas-cohort.json")?;
        evidence::write_new(&destination.join("atlas-cohort.json"), &fs::read(path)?)?;
    }
    if source.join("atlas-labels.json").is_file() {
        let labels = evidence::confined_file(source, "atlas-labels.json")?;
        evidence::write_new(&destination.join("atlas-labels.json"), &fs::read(labels)?)?;
    }
    Ok(())
}

pub(crate) fn set_cohort(root: &Path, manifest: &Path) -> Result<()> {
    let cohort = read_json_file(manifest)?;
    cohort_books(&cohort)?;
    let root = root.canonicalize()?;
    site::write_derived(
        &root,
        &root.join("atlas-cohort.json"),
        &serde_json::to_vec_pretty(&cohort)?,
    )
}

fn cohort_books(cohort: &Value) -> Result<BTreeMap<String, Value>> {
    anyhow::ensure!(cohort["schema"] == 1, "Unsupported Atlas cohort schema");
    let mut books = BTreeMap::new();
    let mut ranks = std::collections::BTreeSet::new();
    for entry in cohort["workbooks"]
        .as_array()
        .context("Cohort requires workbooks")?
    {
        let corpus = entry["corpus"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Missing cohort corpus")?;
        let name = entry["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Missing cohort workbook name")?;
        let original = entry["original_sha256"]
            .as_str()
            .context("Missing original SHA-256")?
            .to_ascii_lowercase();
        anyhow::ensure!(
            original.len() == 64 && original.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid original SHA-256"
        );
        let rank = entry["rank"]
            .as_u64()
            .filter(|rank| *rank > 0)
            .context("Invalid cohort rank")?;
        anyhow::ensure!(ranks.insert((corpus, rank)), "Duplicate cohort rank");
        let id = digest(&json!([corpus, name, original]))?;
        let book = json!({"id":id,"name":name,"corpus":corpus,"rank":rank,
            "original_identity":original,"inventory_complete":false,"sheets":[],"observations":[],
            "cohort_entry":entry});
        anyhow::ensure!(books.insert(id, book).is_none(), "Duplicate cohort workbook");
    }
    Ok(books)
}

pub(super) fn write(root: &Path, run: &Value) -> Result<std::path::PathBuf> {
    let canonical = root.canonicalize()?;
    let root = canonical.as_path();
    for directory in [
        "atlas-assets",
        "atlas-assets/vendor",
        "atlas-data",
        "atlas-thumbnails",
    ] {
        fs::create_dir_all(root.join(directory))?;
        site::ensure_owned_directory(root, &root.join(directory))?;
    }
    for (name, content) in ASSETS {
        site::write_derived(
            root,
            &root.join("atlas-assets").join(name),
            content.as_bytes(),
        )?;
    }
    let labels = if root.join("atlas-labels.json").is_file() {
        read_json_file(&evidence::confined_file(root, "atlas-labels.json")?)?
    } else {
        Value::Null
    };
    let mut books = if root.join("atlas-cohort.json").is_file() {
        cohort_books(&read_json_file(&evidence::confined_file(root, "atlas-cohort.json")?)?)?
    } else {
        BTreeMap::new()
    };
    let mut variants = BTreeMap::<String, Value>::new();
    for pair in run["pairs"].as_array().context("Atlas run has no pairs")? {
        append_observation(root, run, pair, &labels, &mut books, &mut variants)?;
    }
    let bytes = walkdir::WalkDir::new(root)
        .into_iter()
        .try_fold(0u64, |total, entry| {
            let entry = entry?;
            Ok::<_, anyhow::Error>(
                total
                    + if entry.file_type().is_file() {
                        entry.metadata()?.len()
                    } else {
                        0
                    },
            )
        })?;
    let mut books: Vec<_> = books.into_values().collect();
    books.sort_by(|a, b| {
        a["corpus"].as_str().cmp(&b["corpus"].as_str())
            .then(a["rank"].as_u64().unwrap_or(u64::MAX).cmp(&b["rank"].as_u64().unwrap_or(u64::MAX)))
            .then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let catalog = json!({"schema":1,"run_id":run["run_id"],"status":run["status"],
        "storage":{"bytes_before_index":bytes,"github_pages_budget_bytes":1_000_000_000u64,
            "within_budget":bytes + 1024 * 1024 < 1_000_000_000u64},
        "books":books,"variants":variants.into_values().collect::<Vec<_>>(),
        "scope":"Recorded observations under each source run's selection policy. Neutral sheets do not establish whole-sheet pixel equality."});
    write_gzip(root, "atlas-data/catalog.json.gz", &catalog)?;
    let html = include_str!("../../office-capture/sheet-atlas/index.html").replace(
        "{{PUBLICATION_STATUS}}",
        if root.join("publication-review.json").is_file() {
            "publication_review_required"
        } else {
            "local_report"
        },
    );
    let path = root.join("index.html");
    site::write_derived(root, &path, html.as_bytes())?;
    site::write_derived(root, &root.join(".nojekyll"), b"")?;
    Ok(path)
}

fn append_observation(
    root: &Path,
    run: &Value,
    pair: &Value,
    labels: &Value,
    books: &mut BTreeMap<String, Value>,
    variants: &mut BTreeMap<String, Value>,
) -> Result<()> {
    let source = source_metadata(run, pair);
    let run_id = source["run_id"].as_str().unwrap_or("unknown");
    let label = labels.get(run_id).or_else(|| {
        pair["merge_source"]["source_run_sha256"]
            .as_str()
            .and_then(|id| labels.get(id))
    });
    let atlas = label.unwrap_or(&source["config"]["atlas"]);
    let corpus = atlas["corpus"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unclassified");
    let variant_name = atlas["variant"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| source["config"]["steps"].as_array()?.last()?["name"].as_str())
        .unwrap_or("Candidate");
    // Explicit presentation groups share a selector entry, never an observation.
    let presentation_group = atlas["variant_group"].as_str().filter(|group| !group.is_empty());
    let variant = if let Some(group) = presentation_group {
        digest(&json!(["presentation-group", group, variant_name]))?
    } else {
        digest(&json!([run_id, source["validation_signature"], source["config"]]))?
    };
    let variant_entry = variants
        .entry(variant.clone())
        .or_insert_with(|| json!({"id":variant,"name":variant_name,"run_id":run_id,
            "presentation_group":presentation_group,"run_ids":[]}));
    let run_ids = variant_entry["run_ids"].as_array_mut().unwrap();
    if !run_ids.iter().any(|id| id == run_id) {
        run_ids.push(json!(run_id));
    }
    let name = pair["name"].as_str().unwrap_or("Unnamed workbook");
    let display = pair["display_name"].as_str().unwrap_or(name);
    let suffix = pair["merge_source"]["source_run_sha256"]
        .as_str()
        .and_then(|hash| hash.get(..8))
        .map(|hash| format!(" · run {hash}"));
    let display = suffix
        .as_deref()
        .and_then(|suffix| display.strip_suffix(suffix))
        .unwrap_or(display);
    let relative = pair["report"].as_str();
    let report = relative
        .map(|p| evidence::confined_file(root, p).and_then(|p| read_json_file(&p)))
        .transpose()?;
    let inventory = report
        .as_ref()
        .and_then(|r| r.get("workbook_inventory"))
        .filter(|v| v["complete"] == true)
        .cloned()
        .or_else(|| pair.get("workbook_inventory").filter(|v| v["complete"] == true).cloned())
        .or_else(|| inventory::recover(&source["config"], name));
    let original = original_identity(root, pair, inventory.as_ref())?;
    let book_id = digest(&json!([corpus, name, original]))?;
    let book = books.entry(book_id.clone()).or_insert_with(|| json!({
        "id":book_id,"name":display,"corpus":corpus,
        "original_identity":original,"inventory_complete":inventory.as_ref().is_some_and(|v| v["complete"]==true),
        "sheets":inventory.as_ref().map(|v|v["sheets"].clone()).unwrap_or(json!([])),"observations":[]
    }));
    if book["inventory_complete"] != true {
        if let Some(inventory) = inventory.as_ref() {
            if inventory["complete"] == true {
                book["sheets"] = inventory["sheets"].clone();
                book["inventory_complete"] = json!(true);
            }
        }
    }
    let report_digest = report.as_ref().map(digest).transpose()?;
    let observation_id = digest(&json!([variant, pair["id"], relative, report_digest]))?;
    let mut summary = json!({"id":observation_id,"variant":variant,"run_id":run_id,"status":pair["status"],
        "errors":pair["errors"],"error":pair["error"],"sheets":{}});
    if let (Some(relative), Some(report)) = (relative, report) {
        let parent = Path::new(relative)
            .parent()
            .context("Missing report parent")?
            .to_string_lossy()
            .replace('\\', "/");
        let detail = format!("atlas-data/{observation_id}.json.gz");
        write_gzip(
            root,
            &detail,
            &json!({"schema":1,"image_base":format!("{parent}/"),"report":report,
            "validation_signature":source["validation_signature"],"run_id":run_id}),
        )?;
        summary["detail"] = json!(detail);
        summary["page"] = json!(format!("{parent}/index.html"));
        let mut sheets = BTreeMap::<String, Value>::new();
        for coverage in report["content_coverage"].as_array().into_iter().flatten() {
            if let Some(name) = coverage["sheet"].as_str() {
                let bounds = &coverage["content_envelope_with_one_cell_margin"];
                sheets.insert(name.to_owned(), json!({"differences":0,"captures":0,"rows":bounds["bottom"],"columns":bounds["right"]}));
            }
        }
        for comparison in report["comparisons"].as_array().into_iter().flatten() {
            if !matches!(comparison["left"].as_str(), Some("O" | "OR"))
                || !matches!(comparison["right"].as_str(), Some("SD" | "SDR"))
            {
                continue;
            }
            for finding in ["findings", "charts"]
                .into_iter()
                .flat_map(|key| comparison[key].as_array().into_iter().flatten())
            {
                let Some(name) = finding["sheet"].as_str() else {
                    continue;
                };
                let sheet = sheets
                    .entry(name.to_owned())
                    .or_insert_with(|| json!({"differences":0,"captures":0}));
                if super::is_difference(finding) {
                    sheet["differences"] = json!(sheet["differences"].as_u64().unwrap_or(0) + 1);
                }
                if finding["pixels_compared"] == true {
                    sheet["captures"] = json!(sheet["captures"].as_u64().unwrap_or(0) + 1);
                }
                if sheet["thumbnail"].is_null() {
                    if let Some(image) = finding["left_png"]
                        .as_str()
                        .or(finding["left_image"].as_str())
                    {
                        sheet["thumbnail"] = thumbnail(root, &parent, image)?
                            .map(Value::String)
                            .unwrap_or(Value::Null);
                    }
                }
            }
        }
        for name in sheets.keys() {
            if !book["sheets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == *name)
            {
                let order = book["sheets"].as_array().unwrap().len();
                book["sheets"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":name,"name":name,"state":"unknown","order":order}));
            }
        }
        summary["sheets"] = serde_json::to_value(sheets)?;
    }
    book["observations"].as_array_mut().unwrap().push(summary);
    Ok(())
}

fn source_metadata<'a>(run: &'a Value, pair: &Value) -> &'a Value {
    let mut merge = &run["merge"];
    let mut reference = &pair["merge_source"];
    let mut source = run;
    while let Some(next) = merge["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["source_run_sha256"] == reference["source_run_sha256"])
    {
        source = next;
        merge = &source["previous_merge"];
        reference = &reference["previous"];
    }
    source
}

fn original_identity(root: &Path, pair: &Value, inventory: Option<&Value>) -> Result<String> {
    if let Some(hash) = inventory.and_then(|v| v["original_sha256"].as_str()) {
        return Ok(hash.to_owned());
    }
    if let Some(relative) = pair["identity"].as_str() {
        let identity = read_json_file(&evidence::confined_file(root, relative)?)?;
        // Partial fingerprints cannot establish that two complete originals match.
        if [
            "other",
            "worksheets",
            "workbook",
            "styles",
            "shared_strings",
            "relationships",
            "drawings",
            "charts",
        ]
        .iter()
        .all(|key| identity["selection"][key] == true)
        {
            return digest(&json!([identity["selection"], identity["original"]]));
        }
    }
    digest(&json!(["unclassified", pair["merge_source"], pair["id"]]))
}

fn thumbnail(root: &Path, parent: &str, image: &str) -> Result<Option<String>> {
    let source = evidence::confined_file(root, &format!("{parent}/{image}"))?;
    let hash = deltasieve_engine::reuse::file_sha256(&source)?;
    let relative = format!("atlas-thumbnails/{hash}.png");
    if root.join(&relative).is_file() {
        evidence::confined_file(root, &relative)?;
        return Ok(Some(relative));
    }
    let mut reader = image::ImageReader::open(source)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let Ok(decoded) = reader.decode() else {
        return Ok(None);
    };
    let preview = decoded.thumbnail(480, 320);
    let mut bytes = Cursor::new(Vec::new());
    preview.write_to(&mut bytes, image::ImageFormat::Png)?;
    site::write_derived(root, &root.join(&relative), bytes.get_ref())?;
    Ok(Some(relative))
}

fn digest(value: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn write_gzip(root: &Path, relative: &str, value: &Value) -> Result<()> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&serde_json::to_vec(value)?)?;
    site::write_derived(root, &root.join(relative), &encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn catalog(root: &Path) -> Result<Value> {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(fs::File::open(root.join("atlas-data/catalog.json.gz"))?)
            .read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    #[test]
    fn nested_collections_keep_the_original_corpus_and_processor_labels() {
        let run = json!({"merge":{"sources":[{"source_run_sha256":"outer","run_id":"collection",
            "previous_merge":{"sources":[{"source_run_sha256":"inner","run_id":"original-run",
                "config":{"atlas":{"corpus":"Destatis","variant":"Processor"}}}]}}]}});
        let pair = json!({"merge_source":{"source_run_sha256":"outer",
            "previous":{"source_run_sha256":"inner"}}});
        let source = source_metadata(&run, &pair);
        assert_eq!(source["run_id"], "original-run");
        assert_eq!(source["config"]["atlas"]["corpus"], "Destatis");
    }

    #[test]
    fn collection_groups_originals_and_retains_blank_sheets_and_independent_variants() -> Result<()>
    {
        let root = std::env::temp_dir().join(format!("atlas-contract-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("raw"))?;
        let raw = json!({"status":"completed_with_limitations","workbook_inventory":{
            "complete":true,"original_sha256":"0".repeat(64),"sheets":[
                {"id":"1","name":"Data","order":0,"state":"visible"},
                {"id":"2","name":"Empty","order":1,"state":"hidden"}]},
            "comparisons":[{"left":"OR","right":"SDR","findings":[
                {"sheet":"Data","cell":"A1","presence_differs":true}]},
                {"left":"O","right":"OR","findings":[
                    {"sheet":"Empty","cell":"A1","pixels_differ":true}]}]});
        fs::write(root.join("raw/report.json"), serde_json::to_vec(&raw)?)?;
        super::super::package_comparison(&root.join("raw"), &root.join("pairs/one"))?;
        let original_bytes = fs::read(root.join("pairs/one/report.json"))?;
        let run = json!({"run_id":"collection","merge":{"sources":[
            {"source_run_sha256":"a","run_id":"run-a","config":{"atlas":{"corpus":"Corpus","variant":"Tool A"}}},
            {"source_run_sha256":"b","run_id":"run-b","config":{"atlas":{"corpus":"Corpus","variant":"Tool B"}}}]},
            "pairs":[
                {"id":"one","name":"book.xlsx","status":"completed_with_limitations","report":"pairs/one/report.json","merge_source":{"source_run_sha256":"a"}},
                {"id":"two","name":"book.xlsx","status":"completed_with_limitations","report":"pairs/one/report.json","merge_source":{"source_run_sha256":"b"}}]});
        fs::write(root.join("atlas-cohort.json"), serde_json::to_vec(&json!({
            "schema":1,"workbooks":[{"corpus":"Corpus","name":"book.xlsx",
                "rank":1,"original_sha256":"0".repeat(64)}]
        }))?)?;
        write(&root, &run)?;
        let catalog = catalog(&root)?;
        assert_eq!(catalog["books"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["variants"].as_array().unwrap().len(), 2);
        let book = &catalog["books"][0];
        assert_eq!(book["sheets"][1]["name"], "Empty");
        assert_eq!(book["inventory_complete"], true);
        assert_eq!(book["observations"].as_array().unwrap().len(), 2);
        for observation in book["observations"].as_array().unwrap() {
            assert_eq!(observation["sheets"]["Data"]["differences"], 1);
            assert!(observation["sheets"]["Empty"].is_null());
            assert!(root.join(observation["detail"].as_str().unwrap()).is_file());
        }
        assert_eq!(
            original_bytes,
            fs::read(root.join("pairs/one/report.json"))?
        );
        super::super::verify_comparison(&root.join("pairs/one"))?;
        fs::write(root.join("atlas-labels.json"), serde_json::to_vec(&json!({
            "run-a":{"corpus":"Corpus","variant":"Shared tool","variant_group":"tool-profile"},
            "run-b":{"corpus":"Corpus","variant":"Shared tool","variant_group":"tool-profile"}
        }))?)?;
        write(&root, &run)?;
        let grouped = self::catalog(&root)?;
        assert_eq!(grouped["variants"].as_array().unwrap().len(), 1);
        assert_eq!(grouped["variants"][0]["run_ids"], json!(["run-a", "run-b"]));
        let observations = grouped["books"][0]["observations"].as_array().unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0]["run_id"], "run-a");
        assert_eq!(observations[1]["run_id"], "run-b");
        assert_ne!(observations[0]["id"], observations[1]["id"]);
        assert_eq!(original_bytes, fs::read(root.join("pairs/one/report.json"))?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn frozen_cohort_keeps_unobserved_workbooks_in_rank_order() -> Result<()> {
        let root = std::env::temp_dir().join(format!("atlas-cohort-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let entries = json!([
            {"corpus":"Corpus","name":"second.xlsx","rank":2,"original_sha256":"2".repeat(64)},
            {"corpus":"Corpus","name":"first.xlsx","rank":1,"original_sha256":"1".repeat(64)}
        ]);
        let manifest = json!({"schema":1,"workbooks":entries});
        fs::write(root.join("atlas-cohort.json"), serde_json::to_vec(&manifest)?)?;
        write(&root, &json!({"run_id":"empty","pairs":[]}))?;
        let catalog = catalog(&root)?;
        let books = catalog["books"].as_array().unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0]["name"], "first.xlsx");
        assert_eq!(books[1]["rank"], 2);
        assert_eq!(books[0]["inventory_complete"], false);
        assert!(books[0]["observations"].as_array().unwrap().is_empty());
        assert!(books[0]["sheets"].as_array().unwrap().is_empty());
        let mut invalid = manifest.clone();
        invalid["workbooks"][1]["rank"] = json!(2);
        assert!(cohort_books(&invalid).is_err());
        invalid = manifest.clone();
        invalid["workbooks"][0]["original_sha256"] = json!("invalid");
        assert!(cohort_books(&invalid).is_err());
        invalid = manifest;
        invalid["workbooks"][1] = invalid["workbooks"][0].clone();
        invalid["workbooks"][1]["rank"] = json!(3);
        assert!(cohort_books(&invalid).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn legacy_failures_keep_unknown_inventory_instead_of_claiming_empty_workbooks() -> Result<()> {
        let root = std::env::temp_dir().join(format!("atlas-failure-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        let run = json!({"run_id":"legacy","pairs":[{"id":"failed","name":"missing.xlsx","status":"failed","error":"timeout"}]});
        write(&root, &run)?;
        let catalog = catalog(&root)?;
        assert_eq!(catalog["books"][0]["inventory_complete"], false);
        assert_eq!(catalog["books"][0]["observations"][0]["status"], "failed");
        assert!(catalog["books"][0]["observations"][0]["detail"].is_null());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
