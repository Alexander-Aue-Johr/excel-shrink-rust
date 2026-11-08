use super::*;
use image::{Rgba, RgbaImage};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("sheet-scope-report-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self(path.canonicalize()?))
    }

    fn directory(&self, name: &str) -> Result<PathBuf> {
        let path = self.0.join(name);
        fs::create_dir_all(&path)?;
        Ok(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_report(directory: &Path, report: &Value) -> Result<()> {
    fs::write(directory.join("report.json"), serde_json::to_vec(report)?)?;
    Ok(())
}

fn report_with_pair(left: &Path, right: &Path) -> Value {
    json!({
        "status": "completed_with_limitations",
        "capture_policy": "fixture-policy",
        "limitations": ["Selected observations only"],
        "comparisons": [{
            "left": "OR", "right": "SDR", "label": "rebuilt pair",
            "findings": [{"sheet": "Data", "cell": "A1:B2", "pixels_differ": true,
                "pixels_compared": true, "left_png": left, "right_png": right}],
            "charts": []
        }],
        "findings": [{"original_recalc_png": left, "shrink_recalc_png": right}],
        "workbook_path": "not-a-retained-image/original.xlsx"
    })
}

fn image_pair(directory: &Path) -> Result<(PathBuf, PathBuf)> {
    let left = directory.join("left capture.png");
    let right = directory.join("right capture.png");
    let image = RgbaImage::from_pixel(2, 2, Rgba([255; 4]));
    image.save(&left)?;
    let mut changed = image;
    changed.put_pixel(1, 1, Rgba([254, 255, 255, 255]));
    changed.save(&right)?;
    Ok((left, right))
}

#[test]
fn package_survives_workbook_and_scratch_removal_with_shared_exact_pixel_mask() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let (left, right) = image_pair(&raw)?;
    write_report(&raw, &report_with_pair(&left, &right))?;
    fs::write(
        raw.join("original.xlsx"),
        b"do not retain working workbooks",
    )?;
    fs::write(
        raw.join("diagnostic.stdout.log"),
        b"unrelated scratch output",
    )?;
    let destination = fixture.0.join("retained");
    let packaged = package_comparison(&raw, &destination)?;
    assert_eq!(packaged.image_count, 3);
    assert_eq!(packaged.different_regions, 1);
    let report = read_json_file(&packaged.report_path)?;
    let finding = &report["comparisons"][0]["findings"][0];
    assert!(finding["left_png"]
        .as_str()
        .unwrap()
        .starts_with("evidence/"));
    assert_eq!(
        finding["left_png"],
        report["findings"][0]["original_recalc_png"]
    );
    let metrics = &report["portable_report"]["diagnostics"]["0:findings:0"]["metrics"];
    assert_eq!(metrics["changed_pixels"], 1);
    assert_eq!(metrics["max_channel_delta"], 1);
    let overlay =
        image::open(destination.join(metrics["overlay_url"].as_str().unwrap()))?.to_rgba8();
    assert_eq!(overlay.get_pixel(1, 1).0, [255, 25, 25, 220]);
    assert_eq!(overlay.get_pixel(0, 0).0, [0; 4]);
    assert!(!destination.join("original.xlsx").exists());
    assert!(!destination.join("diagnostic.stdout.log").exists());
    fs::remove_dir_all(&raw)?;
    verify_comparison(&destination)
}

#[test]
fn successful_report_wins_over_the_last_intermediate_snapshot() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    write_report(
        &raw,
        &json!({"status":"completed_with_limitations","comparisons":[]}),
    )?;
    fs::write(
        raw.join("partial-report.json"),
        br#"{"status":"running","comparisons":[]}"#,
    )?;
    let packaged = package_comparison(&raw, &fixture.0.join("retained"))?;
    let report = read_json_file(&packaged.report_path)?;
    assert_eq!(report["status"], "completed_with_limitations");
    assert_eq!(report["portable_report"]["source_report"], "report.json");
    assert_eq!(packaged.image_count, 0);
    Ok(())
}

#[test]
fn explicit_failure_keeps_partial_outcome_instead_of_a_stale_completion() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    write_report(
        &raw,
        &json!({"status":"completed_with_limitations","comparisons":[]}),
    )?;
    fs::write(
        raw.join("partial-report.json"),
        br#"{"status":"failed","comparisons":[]}"#,
    )?;
    fs::write(
        raw.join("failure.json"),
        br#"{"status":"failed","error":"owned process did not close"}"#,
    )?;
    let packaged = package_comparison(&raw, &fixture.0.join("retained"))?;
    let report = read_json_file(&packaged.report_path)?;
    assert_eq!(report["status"], "failed");
    assert_eq!(
        report["portable_report"]["source_report"],
        "partial-report.json"
    );
    assert_eq!(
        report["portable_report"]["failure"]["error"],
        "owned process did not close"
    );
    Ok(())
}

#[test]
fn explicit_failure_can_retain_a_partial_snapshot_after_an_interrupted_final_write() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    fs::write(
        raw.join("report.json"),
        br#"{"status":"completed_with_limitations","comparisons":["#,
    )?;
    fs::write(
        raw.join("partial-report.json"),
        br#"{"status":"failed","comparisons":[]}"#,
    )?;
    assert!(package_comparison(&raw, &fixture.0.join("unconfirmed")).is_err());
    fs::write(
        raw.join("failure.json"),
        br#"{"status":"failed","error":"writer stopped"}"#,
    )?;
    let packaged = package_comparison(&raw, &fixture.0.join("retained"))?;
    let report = read_json_file(&packaged.report_path)?;
    assert_eq!(report["status"], "failed");
    assert_eq!(
        report["portable_report"]["source_report"],
        "partial-report.json"
    );
    assert_eq!(
        report["portable_report"]["failure"]["error"],
        "writer stopped"
    );
    verify_comparison(packaged.report_path.parent().unwrap())
}

#[test]
fn rejected_nested_destination_does_not_mutate_the_source_report() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    write_report(
        &raw,
        &json!({"status":"completed_with_limitations","comparisons":[]}),
    )?;
    assert!(package_comparison(&raw, &raw.join("nested/export")).is_err());
    assert!(!raw.join("nested").exists());
    Ok(())
}

#[test]
fn jpeg_chart_exports_keep_their_real_format_and_are_not_mislabelled_as_png() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let chart = raw.join("chart.image");
    image::RgbImage::from_pixel(2, 2, image::Rgb([25, 90, 180]))
        .save_with_format(&chart, image::ImageFormat::Jpeg)?;
    write_report(
        &raw,
        &json!({"status":"completed_with_limitations","comparisons":[{
            "left":"OR","right":"SDR","findings":[],"charts":[{
                "sheet":"Chart","chart":"Revenue","left_image":chart,"right_image":chart,
                "pixels_compared":true,"pixels_differ":false
            }]
        }]}),
    )?;
    let packaged = package_comparison(&raw, &fixture.0.join("retained"))?;
    let report = read_json_file(&packaged.report_path)?;
    assert_eq!(packaged.image_count, 1);
    assert!(report["comparisons"][0]["charts"][0]["left_image"]
        .as_str()
        .unwrap()
        .ends_with(".jpg"));
    Ok(())
}

#[test]
fn missing_or_outside_images_cannot_publish_a_complete_bundle() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let (outside, _) = image_pair(&fixture.0)?;
    for (name, image) in [("missing", raw.join("not-there.png")), ("outside", outside)] {
        write_report(&raw, &report_with_pair(&image, &image))?;
        let target = fixture.0.join(name);
        assert!(package_comparison(&raw, &target).is_err());
        assert!(!target.join("bundle.json").exists());
    }
    Ok(())
}

#[test]
fn verified_reuse_copies_evidence_without_decoding_or_changing_its_report() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let (left, right) = image_pair(&raw)?;
    write_report(&raw, &report_with_pair(&left, &right))?;
    let first = fixture.0.join("first");
    let second = fixture.0.join("second");
    package_comparison(&raw, &first)?;
    package_comparison(&first, &second)?;
    assert_eq!(
        fs::read(first.join("report.json"))?,
        fs::read(second.join("report.json"))?
    );
    assert_eq!(
        fs::read(first.join("bundle.json"))?,
        fs::read(second.join("bundle.json"))?
    );
    fs::remove_dir_all(&first)?;
    verify_comparison(&second)
}

#[test]
fn modified_evidence_fails_integrity_before_reuse() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let (left, right) = image_pair(&raw)?;
    write_report(&raw, &report_with_pair(&left, &right))?;
    let directory = fixture.0.join("retained");
    let packaged = package_comparison(&raw, &directory)?;
    let report = read_json_file(&packaged.report_path)?;
    let image = report["comparisons"][0]["findings"][0]["left_png"]
        .as_str()
        .unwrap();
    fs::write(directory.join(image), b"changed after publication")?;
    assert!(verify_comparison(&directory).is_err());
    let destination = fixture.0.join("bad-reuse");
    assert!(package_comparison(&directory, &destination).is_err());
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn static_export_retains_reuse_identity_and_never_changes_the_source_report() -> Result<()> {
    let fixture = Fixture::new()?;
    let raw = fixture.directory("raw")?;
    let root = fixture.directory("run")?;
    write_report(
        &raw,
        &json!({"status":"completed_with_limitations","comparisons":[]}),
    )?;
    package_comparison(&raw, &root.join("pairs/one"))?;
    fs::create_dir(root.join("identities"))?;
    fs::write(
        root.join("identities/one.json"),
        br#"{"parts":["worksheets"],"original":"fixture"}"#,
    )?;
    let run = json!({"schema":1,"tool":"DeltaSieve","status":"completed","index_complete":true,
        "pairs":[{"id":"one","name":"Book & <one>.xlsx","status":"completed_with_limitations",
            "report":"pairs/one/report.json","identity":"identities/one.json","different_regions":0}]});
    fs::write(root.join("run.json"), serde_json::to_vec(&run)?)?;
    let before = fs::read(root.join("run.json"))?;
    let target = fixture.0.join("website");
    let page = export_site(&root, &target)?;
    assert!(page.is_file());
    assert!(target.join("pairs/one/index.html").is_file());
    assert!(target.join("report-assets/report-view.js").is_file());
    assert_eq!(
        fs::read(root.join("identities/one.json"))?,
        fs::read(target.join("identities/one.json"))?
    );
    assert_eq!(before, fs::read(root.join("run.json"))?);
    assert!(!root.join("index.html").exists());
    Ok(())
}

#[test]
fn portable_paths_reject_traversal_and_windows_absolute_syntax_on_every_host() {
    for path in [
        "../report.json",
        "/report.json",
        "a//report.json",
        "a/./report.json",
        "C:/report.json",
        "a\\report.json",
        "a/../../report.json",
        "a/\u{0}report.json",
    ] {
        assert!(evidence::relative_path(path).is_err(), "accepted {path:?}");
    }
    assert!(evidence::relative_path("pairs/one/report.json").is_ok());
}

#[test]
fn recorded_identity_digest_prevents_export_of_changed_reuse_metadata() -> Result<()> {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new()?;
    let source = fixture.directory("run")?;
    fs::create_dir(source.join("identities"))?;
    let relative = "identities/one.json";
    let bytes = br#"{"parts":["worksheets"]}"#;
    fs::write(source.join(relative), bytes)?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let valid = fixture.directory("valid")?;
    evidence::copy_identity(&source, &valid, relative, Some(&digest))?;
    fs::write(source.join(relative), br#"{"parts":["styles"]}"#)?;
    let changed = fixture.directory("changed")?;
    assert!(evidence::copy_identity(&source, &changed, relative, Some(&digest)).is_err());
    assert!(!changed.join(relative).exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn image_symlinks_cannot_escape_a_retained_package() -> Result<()> {
    let fixture = Fixture::new()?;
    let root = fixture.directory("root")?;
    let outside = fixture.0.join("outside.png");
    fs::write(&outside, b"outside")?;
    std::os::unix::fs::symlink(&outside, root.join("linked.png"))?;
    assert!(evidence::confined_file(&root, "linked.png").is_err());
    Ok(())
}
