//! Adapt completed native reference receipts for the ordinary static viewer.
use super::{evidence, inventory, package_comparison, site};
use anyhow::{ensure, Context, Result};
use clap::Args;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

#[derive(Debug, Args)]
pub(crate) struct ImportReference {
    pub reference: PathBuf,
    pub output: PathBuf,
    #[arg(long)]
    pub workbook: String,
    #[arg(long)]
    pub corpus: String,
    #[arg(long)]
    pub variant: String,
    /// Explain how this reference was selected, including posthoc selection.
    #[arg(long)]
    pub scope_note: String,
}

pub(crate) fn import(args: ImportReference) -> Result<()> {
    let source = args.reference.canonicalize()?;
    let receipt_path = evidence::confined_file(&source, "reference.json")?;
    let receipt_bytes = fs::read(&receipt_path)?;
    let receipt: Value = serde_json::from_slice(&receipt_bytes)?;
    ensure!(
        receipt["version"] == 1 && receipt["status"] == "completed",
        "Reference receipt is not complete"
    );
    ensure!(
        receipt["selection"] == "explicit_independent_reference_plan",
        "Unsupported reference selection"
    );
    ensure!(
        !args.scope_note.trim().is_empty(),
        "Reference selection scope is required"
    );
    for (file, key) in [
        ("original.xlsx", "original_rebuilt_sha256"),
        ("default.xlsx", "default_rebuilt_sha256"),
    ] {
        let path = evidence::confined_file(&source, file)?;
        ensure!(
            deltasieve_engine::reuse::file_sha256(&path)? == receipt[key],
            "Retained reference workbook differs"
        );
    }
    let mut workbook_inventory = inventory::read(&source.join("original.xlsx"))?;
    let original_sha = receipt["original_sha256"]
        .as_str()
        .context("Missing original source SHA-256")?;
    ensure!(
        original_sha.len() == 64 && original_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid original source SHA-256"
    );
    workbook_inventory["original_sha256"] = json!(original_sha);
    let mut findings = Vec::new();
    for region in receipt["regions"]
        .as_array()
        .context("Reference regions missing")?
    {
        let images = region["images"]
            .as_array()
            .context("Reference images missing")?;
        ensure!(
            images.len() == 2 && region["pixels_compared"] == true,
            "Reference has no complete pixel comparison"
        );
        for image in images {
            let relative = image["file"]
                .as_str()
                .context("Reference image path missing")?;
            let path = evidence::confined_file(&source, relative)?;
            ensure!(
                fs::metadata(&path)?.len()
                    == image["bytes"]
                        .as_u64()
                        .context("Missing image byte length")?
                    && deltasieve_engine::reuse::file_sha256(&path)? == image["sha256"],
                "Reference image differs from receipt"
            );
        }
        findings.push(json!({"sheet":region["sheet"],"cell":region["range"],
            "left_geometry":region["original_geometry"],"right_geometry":region["default_geometry"],
            "left_png":evidence::confined_file(&source, images[0]["file"].as_str().unwrap())?,
            "right_png":evidence::confined_file(&source, images[1]["file"].as_str().unwrap())?,
            "pixels_compared":region["pixels_compared"],"pixels_differ":region["pixels_differ"],
            "geometry_differs":region["geometry_differs"],"rendering_differs":region["rendering_differs"]}));
    }
    ensure!(!findings.is_empty(), "Reference has no completed regions");
    let output = evidence::create_destination(&source, &args.output)?;
    evidence::write_new(&output.join(".incomplete"), b"Reference import in progress")?;
    let staging = output.join("reference-staging");
    fs::create_dir(&staging)?;
    // The packager requires images below its source directory. Copy only the
    // images whose receipt identities were checked above, then recheck the copy.
    for finding in &mut findings {
        for key in ["left_png", "right_png"] {
            let path = PathBuf::from(finding[key].as_str().context("Missing staged image")?);
            let hash = deltasieve_engine::reuse::file_sha256(&path)?;
            let target = staging.join(format!("{hash}.png"));
            if !target.exists() {
                fs::copy(&path, &target)?;
            }
            ensure!(
                deltasieve_engine::reuse::file_sha256(&target)? == hash,
                "Reference image copy differs"
            );
            finding[key] = json!(target);
        }
    }
    let receipt_sha = deltasieve_engine::reuse::file_sha256(&receipt_path)?;
    let report = json!({"status":"completed_with_limitations","capture_policy":"explicit-reference-plan-v1",
        "selection":{"kind":"explicit_reference_import","scope_note":args.scope_note,"recall_evidence":false},
        "limitations":[args.scope_note,"Supplied reference areas do not establish whole-sheet coverage or selector recall."],
        "preserve_image_encoding":true,
        "reference_provenance":{"source_receipt_sha256":receipt_sha,"source_receipt_json":std::str::from_utf8(&receipt_bytes)?},
        "workbook_inventory":workbook_inventory,
        "comparisons":[{"left":"OR","right":"SDR","label":"Explicit reference after native rebuild",
            "selected_region_count":findings.len(),"findings":findings,"charts":[]}]});
    fs::write(
        staging.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let pair = output.join("pairs/reference");
    package_comparison(&staging, &pair)?;
    // This is an owned staging directory created within this new output only.
    fs::remove_dir_all(&staging)?;
    fs::write(output.join("source-reference.json"), &receipt_bytes)?;
    let run = json!({"schema":1,"tool":"DeltaSieve","run_id":format!("reference-{receipt_sha}"),
        "status":"completed_with_limitations","index_complete":true,"started_at":receipt["started_at"],"finished_at":receipt["finished_at"],
        "selection":report["selection"],"config":{"atlas":{"corpus":format!("{} (explicit reference controls)", args.corpus),"variant":args.variant}},
        "validation_signature":{"capture_policy":"explicit-reference-plan-v1","producer_sha256":receipt["producer_sha256"]},
        "summary":{"total":1,"completed":1,"failed":0},
        "pairs":[{"id":"reference","name":args.workbook,"status":"completed_with_limitations",
            "report":"pairs/reference/report.json","different_regions":super::difference_count(&report)}]});
    fs::write(output.join("run.json"), serde_json::to_vec_pretty(&run)?)?;
    site::write_site(&output, &run)?;
    ensure!(
        fs::read(&receipt_path)? == receipt_bytes,
        "Source receipt changed during import"
    );
    fs::remove_file(output.join(".incomplete"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reference_import_preserves_scope_images_and_source_and_rejects_corruption() -> Result<()> {
        let root = std::env::temp_dir().join(format!("reference-import-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        let mut zip = zip::ZipWriter::new(fs::File::create(source.join("original.xlsx"))?);
        zip.start_file("xl/workbook.xml", zip::write::FileOptions::default())?;
        zip.write_all(br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Tables" sheetId="1" r:id="rId1"/></sheets></workbook>"#)?;
        zip.finish()?;
        fs::copy(source.join("original.xlsx"), source.join("default.xlsx"))?;
        let workbook_hash = deltasieve_engine::reuse::file_sha256(&source.join("original.xlsx"))?;
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255u8; 4]));
        image.save(source.join("left.png"))?;
        image.save(source.join("right.png"))?;
        let images: Vec<_> = ["left.png", "right.png"]
            .into_iter()
            .map(|name| {
                let path = source.join(name);
                Ok(json!({"file":name,"bytes":fs::metadata(&path)?.len(),
                "sha256":deltasieve_engine::reuse::file_sha256(&path)?}))
            })
            .collect::<Result<_>>()?;
        let receipt = json!({"version":1,"status":"completed","selection":"explicit_independent_reference_plan",
            "original_sha256":workbook_hash,"original_rebuilt_sha256":workbook_hash,"default_rebuilt_sha256":workbook_hash,
            "producer_sha256":"fixture-producer","regions":[{"sheet":"Tables","range":"A1:B2",
                "pixels_compared":true,"pixels_differ":false,"geometry_differs":false,
                "original_geometry":{"left":0,"top":0,"width":2,"height":2},
                "default_geometry":{"left":0,"top":0,"width":2,"height":2},"images":images}]});
        let bytes = serde_json::to_vec_pretty(&receipt)?;
        fs::write(source.join("reference.json"), &bytes)?;
        let args = |name: &str| ImportReference {
            reference: source.clone(),
            output: root.join(name),
            workbook: "book.xlsx".into(),
            corpus: "posthoc controls".into(),
            variant: "Processor".into(),
            scope_note: "Selected after observing a finding. Not a recall sample.".into(),
        };
        import(args("report"))?;
        let run = super::super::read_json_file(&root.join("report/run.json"))?;
        assert_eq!(run["selection"]["recall_evidence"], false);
        assert_eq!(run["status"], "completed_with_limitations");
        assert_eq!(
            run["config"]["atlas"]["corpus"],
            "posthoc controls (explicit reference controls)"
        );
        super::super::verify_comparison(&root.join("report/pairs/reference"))?;
        let report =
            super::super::read_json_file(&root.join("report/pairs/reference/report.json"))?;
        assert_eq!(report["equal_image_encoding"]["compression"], "unchanged");
        assert_eq!(
            report["workbook_inventory"]["original_sha256"],
            workbook_hash
        );
        let retained: Value = serde_json::from_str(
            report["reference_provenance"]["source_receipt_json"]
                .as_str()
                .unwrap(),
        )?;
        assert_eq!(retained, receipt);
        let image = report["comparisons"][0]["findings"][0]["left_png"]
            .as_str()
            .unwrap();
        assert_eq!(
            fs::read(root.join("report/pairs/reference").join(image))?,
            fs::read(source.join("left.png"))?
        );
        assert_eq!(fs::read(source.join("reference.json"))?, bytes);
        assert!(root.join("report/index.html").is_file());
        assert!(!root.join("report/.incomplete").exists());
        assert!(import(args("report")).is_err());
        fs::write(source.join("left.png"), b"corrupted after capture")?;
        assert!(import(args("corrupt")).is_err());
        assert!(!root.join("corrupt").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
