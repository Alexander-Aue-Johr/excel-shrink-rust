//! Run the staged comparison once, retaining its reports independently of disposable work.
use super::*;
use std::collections::BTreeSet;
use std::process::{Command, Stdio};
mod retention;

const POLICY: &str = crate::regression::targeted_comparison::CAPTURE_POLICY;

/// A targeted result completes this selection policy, never a whole-sheet visual certificate.
pub(in crate::regression::archive::study) fn complete(
    record: &Value,
    skip_performance: bool,
) -> bool {
    record["complete"] == true
        && record["targeted_comparison"]["policy"] == POLICY
        && record["targeted_comparison"]["skip_check_performance"] == skip_performance
        && record["targeted_comparison"]["status"] == "completed_with_limitations"
}

/// Launch an owned private desktop, then publish only a positively completed report.
pub(super) fn compare(job: &MemberJob, record: &mut Value) -> Result<()> {
    let outcome = execute(job, record);
    if let Err(error) = &outcome {
        record["targeted_comparison"]["status"] = json!("failed");
        record["targeted_comparison"]["error"] = json!(format!("{error:#}"));
    }
    outcome
}

fn execute(job: &MemberJob, record: &mut Value) -> Result<()> {
    let skip = job
        .run_manifest
        .as_ref()
        .is_some_and(|run| run["targeted_skip_check_performance"] == true);
    let key = artifact_cache::sha256(job.identity.as_bytes());
    let directory = job
        .report_root
        .join("evidence")
        .join(&key[..20])
        .join(format!("targeted-{}", uuid::Uuid::new_v4()));
    let group = job
        .control_room
        .as_ref()
        .map(|root| root.join("groups").join(&key[..20]))
        .unwrap_or_else(|| job.work.join("targeted-group"));
    fs::create_dir_all(&group)?;
    record["targeted_comparison"] = json!({"policy":POLICY,"status":"running",
        "skip_check_performance":skip,"report_directory":directory});
    record["checks_complete"] = json!(false);
    record["validation_scope"] = json!("XML-selected content regions and all charts; exterior formatting-only regions intentionally excluded");
    write_record(&job.record, record)?;
    work_state::event(
        "comparing",
        "targeted_comparison_started",
        "Running staged targeted comparison",
        false,
        Some("staged Excel comparison"),
        json!({"report_directory":directory}),
    );
    let script = repo_root().join("excel-shrink-cli/office-capture/run-targeted-comparison.ps1");
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Binary")
        .arg(std::env::current_exe()?)
        .arg("-Original")
        .arg(job.work.join("original.xlsx"))
        .arg("-Default")
        .arg(job.work.join("default.xlsx"))
        .arg("-Output")
        .arg(&directory)
        .arg("-ObservatoryGroup")
        .arg(&group)
        .arg("-OperationTimeoutSeconds")
        .arg(std::env::var("EXCEL_SHRINK_CAPTURE_STARTUP_SECONDS").unwrap_or_else(|_| "120".into()))
        .env("EXCEL_SHRINK_TARGETED_WORKBOOK_LABEL", &job.name)
        .stdin(Stdio::null());
    if skip {
        command.arg("-SkipCheckPerformance");
    }
    let status = command
        .status()
        .context("Start targeted private-desktop comparison")?;
    ensure!(
        status.success(),
        "Targeted comparison failed; inspect {}",
        directory.display()
    );
    let mut report: Value = serde_json::from_slice(&fs::read(directory.join("report.json"))?)?;
    ensure!(
        report["status"] == "completed_with_limitations",
        "Targeted report is incomplete"
    );
    let changed = report["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|comparison| {
            comparison["findings"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|finding| {
                    finding["pixels_differ"] == true || finding["geometry_differs"] == true
                })
                || comparison["charts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|finding| {
                        finding["pixels_differ"] == true
                            || finding["geometry_differs"] == true
                            || finding["presence_differs"] == true
                            || finding["rendering_differs"] == true
                    })
        });
    record["targeted_comparison"]["status"] = report["status"].clone();
    record["targeted_comparison"]["candidate_count"] = report["candidate_count"].clone();
    record["targeted_comparison"]["content_coverage"] = report["content_coverage"].clone();
    record["targeted_comparison"]["report"] = json!(directory.join("report.json"));
    let preview_width = record["control_room_attempt"]["settings"]["equal_preview_width"]
        .as_u64()
        .filter(|width| [0, 128, 256, 512, 1024].contains(width))
        .unwrap_or(512) as u32;
    let obsolete_images = retention::apply(&mut report, &directory, preview_width)?;
    artifact_cache::atomic_write(
        &directory.join("report.json"),
        &serde_json::to_vec(&report)?,
    )?;
    publish_portable_evidence(&directory, &job.report_root, report.clone(), record)?;
    retention::discard_originals(&directory, &obsolete_images)?;
    // Evidence and XML transitions are durable; reusable raw inputs remain in the
    // preparation bundle. Keeping six working XLSX copies per result fills disks.
    for entry in fs::read_dir(&directory)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "xlsx")
        {
            fs::remove_file(&path)
                .with_context(|| format!("Remove completed working copy {}", path.display()))?;
        }
    }
    record["targeted_comparison"]["working_copies_retained"] = json!(false);
    record["status"] = json!(if changed { "differences" } else { "unchanged" });
    record["whitespace_diagnosis"] =
        json!({"status":"not_applicable","reason":"staged_targeted_comparison"});
    for profile in record["results"].as_array_mut().into_iter().flatten() {
        profile["checks_complete"] = json!(false);
        profile["image_comparison"] = json!("targeted");
    }
    record.as_object_mut().unwrap().remove("error");
    write_record(&job.record, record)?;
    work_state::event(
        "compared",
        "targeted_comparison_complete",
        "Targeted comparison completed",
        true,
        None,
        json!({"status":record["status"],"candidate_count":report["candidate_count"]}),
    );
    Ok(())
}

/// Keep portable report/image references explicit so the existing verified export includes them.
fn publish_portable_evidence(
    directory: &Path,
    root: &Path,
    mut report: Value,
    record: &mut Value,
) -> Result<()> {
    fn normalize(
        value: &mut Value,
        root: &Path,
        images: &mut BTreeSet<String>,
        audits: &mut BTreeSet<String>,
    ) -> Result<()> {
        match value {
            Value::Object(fields) => {
                for child in fields.values_mut() {
                    normalize(child, root, images, audits)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    normalize(child, root, images, audits)?;
                }
            }
            Value::String(text) => {
                let path = PathBuf::from(&*text);
                if let Ok(relative) = path.strip_prefix(root) {
                    let mut image = path.clone();
                    if path
                        .extension()
                        .is_some_and(|extension| extension == "image")
                    {
                        image.set_extension("png");
                        if !image.is_file() {
                            image::ImageReader::open(&path)?
                                .with_guessed_format()?
                                .decode()?
                                .save(&image)?;
                        }
                    }
                    *text = if image != path {
                        image
                            .strip_prefix(root)?
                            .to_string_lossy()
                            .replace('\\', "/")
                    } else {
                        relative.to_string_lossy().replace('\\', "/")
                    };
                    if image
                        .extension()
                        .is_some_and(|extension| extension == "png")
                    {
                        images.insert(text.clone());
                    }
                    if text.ends_with(".jsonl.gz") {
                        ensure!(
                            image.is_file(),
                            "Missing XML change audit {}",
                            image.display()
                        );
                        audits.insert(text.clone());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut images = BTreeSet::new();
    let mut audits = BTreeSet::new();
    normalize(&mut report, root, &mut images, &mut audits)?;
    let path = directory.join("portable-report.json");
    artifact_cache::atomic_write(&path, &serde_json::to_vec(&report)?)?;
    record["targeted_comparison"]["targeted_report_file"] = json!(path
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/"));
    record["targeted_comparison"]["evidence_images"] = json!(images
        .into_iter()
        .map(|path| json!({"capture_image":path}))
        .collect::<Vec<_>>());
    record["targeted_comparison"]["audit_files"] = json!(audits
        .into_iter()
        .map(|path| json!({"targeted_audit_file":path}))
        .collect::<Vec<_>>());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_evidence_contains_only_retained_images() -> Result<()> {
        let root = artifact_cache::unique_path(&std::env::temp_dir(), "portable-retention")?;
        let directory = root.join("evidence/attempt");
        fs::create_dir_all(&directory)?;
        let left = directory.join("original.png");
        let right = directory.join("default.png");
        image::RgbaImage::from_pixel(256, 128, image::Rgba([10, 20, 30, 255])).save(&left)?;
        fs::copy(&left, &right)?;
        let mut report = json!({"comparisons":[{"findings":[{
            "sheet":"Data","cell":"A1:P128","pixels_differ":false,
            "geometry_differs":false,"left_png":left,"right_png":right}]}]});
        let obsolete = retention::apply(&mut report, &directory, 64)?;
        let mut record = json!({});
        publish_portable_evidence(&directory, &root, report, &mut record)?;
        retention::discard_originals(&directory, &obsolete)?;
        let images = record["targeted_comparison"]["evidence_images"]
            .as_array()
            .unwrap();
        assert_eq!(images.len(), 1);
        assert!(root
            .join(images[0]["capture_image"].as_str().unwrap())
            .is_file());
        let portable: Value =
            serde_json::from_slice(&fs::read(directory.join("portable-report.json"))?)?;
        let finding = &portable["comparisons"][0]["findings"][0];
        assert_eq!(finding["left_png"], finding["right_png"]);
        assert!(!left.exists() && !right.exists());
        fs::remove_dir_all(&root)?;
        Ok(())
    }
    #[test]
    fn identical_default_does_not_skip_a_requested_performance_lane() {
        let mut record = json!({"results":[{"profile":"default"}],
            "shrink_identity":{"default":{"workbook_original_archive_reused":true}},
            "targeted_check_performance_required":true});
        assert!(!finalize_nonvisual_preparation(&mut record));
        record["targeted_check_performance_required"] = json!(false);
        assert!(finalize_nonvisual_preparation(&mut record));
        assert_eq!(record["status"], "crc-identical");
    }
    #[test]
    fn resume_requires_the_current_policy_and_requested_lanes() {
        let mut record = json!({"complete":true,"status":"unchanged"});
        assert!(!complete(&record, false));
        record["targeted_comparison"] = json!({"policy":POLICY,"status":"completed_with_limitations","skip_check_performance":false});
        assert!(complete(&record, false));
        assert!(!complete(&record, true));
        record["targeted_comparison"]["status"] = json!("failed");
        assert!(!complete(&record, false));
    }
}
