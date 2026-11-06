//! Adapter to the existing private-desktop renderer and its independent idle watchdog.

use super::{storage, CaptureOptions};
use crate::regression::{power_shell_path, targeted_comparison};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, time::Duration};

/// Run the existing capture implementation in an owned Windows desktop/job.
/// Its report and process logs survive failure; successful process exit is still
/// checked against the scientific report before callers may mark a pair complete.
pub(super) fn compare(
    original: &Path,
    candidate: &Path,
    work: &Path,
    logs: &Path,
    label: &str,
    options: &CaptureOptions,
) -> Result<()> {
    ensure!(cfg!(windows), "Visual capture requires a native Windows build and installed Microsoft Excel");
    fs::create_dir_all(logs)?;
    let group = logs.join("group");
    fs::create_dir(&group)?;
    let script = capture_directory()?.join("run-targeted-comparison.ps1");
    let mut command = Command::new("powershell.exe");
    command.args(["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(power_shell_path(&script)?)
        .arg("-SheetScope")
        .arg("-Binary").arg(power_shell_path(&std::env::current_exe()?)?)
        .arg("-Original").arg(power_shell_path(original)?)
        .arg("-Default").arg(power_shell_path(candidate)?)
        .arg("-Output").arg(power_shell_path(work)?)
        .arg("-ObservatoryGroup").arg(power_shell_path(&group)?)
        .arg("-SkipCheckPerformance")
        .arg("-OperationTimeoutSeconds").arg(options.operation_timeout_seconds.to_string())
        .arg("-CaptureIdleTimeoutSeconds").arg(options.idle_timeout_seconds.to_string())
        .env("EXCEL_SHRINK_TARGETED_WORKBOOK_LABEL", label)
        // The legacy engine can cache saved/rebuilt XLSX controls globally.
        // Standalone temporary output must own those workbook copies as well.
        .env("EXCEL_SHRINK_REGRESSION_CACHE_DIR", power_shell_path(&work.join("validator-cache"))?)
        .env("EXCEL_SHRINK_CONTROL_ARTIFACT_CACHE_DIR", power_shell_path(&work.join("validator-cache/controls"))?)
        .env("EXCEL_SHRINK_CHECK_PERFORMANCE_CACHE_DIR", power_shell_path(&work.join("validator-cache/performance"))?)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fs::File::create(logs.join("stdout.log"))?))
        .stderr(Stdio::from(fs::File::create(logs.join("stderr.log"))?));
    if options.serial_captures { command.arg("-SerialCaptures"); }
    let outcome = crate::capture_watchdog::run(
        &mut command,
        &work.join("telemetry.json"),
        Duration::from_secs(options.idle_timeout_seconds + 15),
    );
    let receipt = match &outcome {
        Ok(status) => json!({"exit_code":status.code(), "success":status.success(),"launcher_exit_confirmed":true}),
        Err(error) => json!({"exit_code":null,"success":false,"error":format!("{error:#}"),
            "launcher_exit_confirmed":!error.is::<crate::capture_watchdog::CleanupUnconfirmed>()}),
    };
    if let Err(publication) = storage::write_json(&logs.join("outcome.json"), &receipt) {
        return match outcome {
            Err(error) => Err(error.context(format!("Launcher receipt also failed: {publication:#}"))),
            Ok(_) => Err(publication),
        };
    }
    let status = outcome?;
    ensure!(status.success(), "Capture launcher failed; inspect {}", logs.display());
    let report = storage::read_json(&work.join("report.json"))?;
    ensure!(report["status"] == "completed_with_limitations" && !work.join("failure.json").exists(),
        "Capture did not publish a completed scientific report");
    Ok(())
}

pub(super) fn capture_directory() -> Result<PathBuf> {
    let candidates = [
        PathBuf::from("excel-shrink-cli/office-capture"),
        PathBuf::from("office-capture"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("office-capture"),
    ];
    candidates.into_iter().find(|path| path.join("run-targeted-comparison.ps1").is_file())
        .map(fs::canonicalize).transpose()?.context("The Office capture scripts are missing")
}

/// Keep producer releases independent of validator identity. The source digest is
/// embedded at build time; an external processor is deliberately not part of it.
pub(super) fn signature(options: &CaptureOptions, environment_key: &str) -> Result<Value> {
    Ok(json!({
        "schema":1,
        "capture_policy":targeted_comparison::policy().0,
        "execution_strategy":targeted_comparison::policy().1,
        "validator_source_sha256":validator_source_digest(),
        "capture_options":options,
        "control_artifact_cache":"per-pair-temporary-v1",
        "environment_key":environment_key,
        "platform":std::env::consts::OS,
        "architecture":std::env::consts::ARCH,
        "host_recipe_sha256":runtime_recipe_digest()?,
    }))
}

fn runtime_recipe_digest() -> Result<String> {
    let root = capture_directory()?;
    let mut files = std::collections::BTreeMap::new();
    // The host composes PowerShell, C# and Office.js at runtime. Keep edits to
    // those source assets in the reuse identity even without a rebuilt binary.
    for entry in walkdir::WalkDir::new(&root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() { continue; }
        let relative = entry.path().strip_prefix(&root)?;
        if relative.components().any(|part| matches!(part.as_os_str().to_str(),
            Some("control-room" | "tests" | "obj" | "bin"))) { continue; }
        if !entry.path().extension().is_some_and(|ext|
            matches!(ext.to_str(), Some("ps1" | "psm1" | "cs" | "js" | "html" | "xml"))) { continue; }
        files.insert(relative.to_string_lossy().replace('\\', "/"),
            crate::validation_reuse::file_sha256(entry.path())?);
    }
    storage::hash_json(&files)
}

fn validator_source_digest() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for source in VALIDATOR_SOURCES {
        hash.update((source.len() as u64).to_le_bytes());
        hash.update(source);
    }
    format!("{:x}", hash.finalize())
}

// The complete renderer/planner source list is kept separate from orchestration
// and shrinker code. Regenerate this list when adding a comparison source module.
const VALIDATOR_SOURCES: &[&[u8]] = &[
    include_bytes!("../capture_host/classic.rs"),
    include_bytes!("../capture_host/classic_telemetry.rs"),
    include_bytes!("../capture_host/mod.rs"),
    include_bytes!("../capture_host/paired.rs"),
    include_bytes!("../capture_host/resident.rs"),
    include_bytes!("../capture_host/resident_diagnosis_image.rs"),
    include_bytes!("../capture_host/resident_events.rs"),
    include_bytes!("../capture_host/resident_telemetry.rs"),
    include_bytes!("../regression/artifact_cache/scan.rs"),
    include_bytes!("../regression/artifact_cache.rs"),
    include_bytes!("../regression/office_capture/paired_capture.rs"),
    include_bytes!("../regression/office_capture/resident_capture.rs"),
    include_bytes!("../regression/office_capture/resident_capture_diagnosis.rs"),
    include_bytes!("../regression/office_capture/resident_capture_hashes.rs"),
    include_bytes!("../regression/office_capture/resident_capture_live.rs"),
    include_bytes!("../regression/office_capture/resident_capture_pipeline.rs"),
    include_bytes!("../regression/office_capture/resident_capture_sweep.rs"),
    include_bytes!("../regression/office_capture/resident_events.rs"),
    include_bytes!("../regression/office_capture/resident_image_processing.rs"),
    include_bytes!("../regression/office_capture/tall_sheet_analysis.rs"),
    include_bytes!("../regression/office_capture/tall_sheet_selection.rs"),
    include_bytes!("../regression/office_capture.rs"),
    include_bytes!("../regression/office_plan/mod.rs"),
    include_bytes!("../regression/office_plan/streaming_sheet.rs"),
    include_bytes!("../regression/office_plan/visual_plan.rs"),
    include_bytes!("../regression/office_plan/visual_xml.rs"),
    include_bytes!("../regression/row_heights.rs"),
    include_bytes!("../regression/streaming_xml.rs"),
    include_bytes!("../regression/targeted_comparison/candidates.rs"),
    include_bytes!("../regression/targeted_comparison/capture_layout.rs"),
    include_bytes!("../regression/targeted_comparison/cell_stream.rs"),
    include_bytes!("../regression/targeted_comparison/change_evidence.rs"),
    include_bytes!("../regression/targeted_comparison/charts.rs"),
    include_bytes!("../regression/targeted_comparison/check_performance.rs"),
    include_bytes!("../regression/targeted_comparison/column_layout.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/data_bars.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/expression.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/formulas.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/input_independent.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/inventory_audit.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/mod.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/ranges.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/references.rs"),
    include_bytes!("../regression/targeted_comparison/conditional_formatting/xml.rs"),
    include_bytes!("../regression/targeted_comparison/content_bounds.rs"),
    include_bytes!("../regression/targeted_comparison/control_cache.rs"),
    include_bytes!("../regression/targeted_comparison/evidence.rs"),
    include_bytes!("../regression/targeted_comparison/execution_plan.rs"),
    include_bytes!("../regression/targeted_comparison/heights.rs"),
    include_bytes!("../regression/targeted_comparison/host.rs"),
    include_bytes!("../regression/targeted_comparison/image_comparison.rs"),
    include_bytes!("../regression/targeted_comparison/performance_cache.rs"),
    include_bytes!("../regression/targeted_comparison/performance_dependencies.rs"),
    include_bytes!("../regression/targeted_comparison/performance_reuse.rs"),
    include_bytes!("../regression/targeted_comparison/prefix_inspection.rs"),
    include_bytes!("../regression/targeted_comparison/preparation.rs"),
    include_bytes!("../regression/targeted_comparison/reporting.rs"),
    include_bytes!("../regression/targeted_comparison/selection_inspection.rs"),
    include_bytes!("../regression/targeted_comparison/snapshot_cache.rs"),
    include_bytes!("../regression/targeted_comparison/staged.rs"),
    include_bytes!("../regression/targeted_comparison/staged_formulas.rs"),
    include_bytes!("../regression/targeted_comparison/static_workbook_metadata.rs"),
    include_bytes!("../regression/targeted_comparison/style_identity/components.rs"),
    include_bytes!("../regression/targeted_comparison/style_identity/tables.rs"),
    include_bytes!("../regression/targeted_comparison/style_identity.rs"),
    include_bytes!("../regression/targeted_comparison/telemetry.rs"),
    include_bytes!("../regression/targeted_comparison/visual_bounds.rs"),
    include_bytes!("../regression/targeted_comparison/workbooks.rs"),
    include_bytes!("../regression/targeted_comparison/xml_changes.rs"),
    include_bytes!("../regression/targeted_comparison.rs"),
    include_bytes!("../regression/visual_compare.rs"),
    include_bytes!("../regression/worksheet_stream.rs"),
    include_bytes!("../regression/xml_views.rs"),
    include_bytes!("../regression/cf_inventory/scan.rs"),
    include_bytes!("../../../excel-shrink-core/src/error.rs"),
    include_bytes!("../../../excel-shrink-core/src/models.rs"),
    include_bytes!("../../../excel-shrink-core/src/styles.rs"),
    include_bytes!("../../../excel-shrink-core/src/utils.rs"),
    include_bytes!("../../../excel-shrink-core/src/xml.rs"),
    include_bytes!("../../../excel-shrink-core/src/xml/attributes.rs"),
    include_bytes!("../../../excel-shrink-core/src/xml/namespaces.rs"),
    include_bytes!("../../../excel-shrink-core/src/xml/namespaces/header.rs"),
    include_bytes!("../../../excel-shrink-core/src/workbook.rs"),
    include_bytes!("../regression/excel_automation.rs"),
    include_bytes!("../native_process.rs"),
    include_bytes!("../regression/formula_trace/mod.rs"),
    include_bytes!("../regression/formula_trace/syntax.rs"),
    include_bytes!("../regression/formula_trace/workbook.rs"),
    include_bytes!("../../../Cargo.lock"),
    include_bytes!("../regression/cf_inventory.rs"),
];
