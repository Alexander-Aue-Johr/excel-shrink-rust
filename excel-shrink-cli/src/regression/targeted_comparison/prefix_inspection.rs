//! Explicitly sampled visual inspection of namespace-normalization controls.
use super::{evidence, telemetry::LaneId, xml_changes::Snapshot, Args, Candidate, Comparison};
use anyhow::{ensure, Result};
use clap::Parser;
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Debug, Parser)]
pub(crate) struct InspectArgs {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    normalized: PathBuf,
    #[arg(long)]
    shrunk: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

/// Audit every cell through disk streams, then inspect stratified deleted-cell neighborhoods.
/// This experiment does not certify whole-workbook visual equality or change production policy.
pub(crate) fn run(input: InspectArgs) -> Result<()> {
    let args = Args {
        original: input.normalized,
        default: input.shrunk,
        output: input.output,
        host_script: None,
        observatory_group_dir: None,
        workbook_label: Some("Prefixed worksheet inspection".into()),
        operation_timeout_seconds: 120,
        skip_check_performance: true,
        serial_captures: false,
        prepare_artifacts_only: false,
    };
    let mut comparison = Comparison::prepare(&args)?;
    let result = inspect(&mut comparison, &input.source);
    let cleanup = comparison.release_excel_and_wait_for_exit();
    let error = result.as_ref().err().or(cleanup.as_ref().err());
    let status = if error.is_some() {
        "failed"
    } else {
        "sampled_inspection_completed"
    };
    fs::write(
        args.output.join("inspection.json"),
        serde_json::to_vec_pretty(&json!({
            "status": status,
            "coverage": "Eight vertical strata per 16-column band, plus top and bottom; not full visual validation",
            "error": error.map(|error| format!("{error:#}")),
        }))?,
    )?;
    comparison.telemetry.update(|state| {
        state.status = status.into();
        state.finished_at = Some(chrono::Utc::now().timestamp_millis());
    })?;
    result?;
    cleanup
}

fn inspect(comparison: &mut Comparison<'_>, source: &std::path::Path) -> Result<()> {
    let before = Snapshot::read(&comparison.paths.original)?;
    let after = Snapshot::read(&comparison.paths.default)?;
    ensure!(
        before.sheets.keys().eq(after.sheets.keys()),
        "Sheet names changed"
    );
    let mut candidates = BTreeSet::new();
    let mut audits = Vec::new();
    for (name, sheet) in &before.sheets {
        let other = &after.sheets[name];
        let (mut removed_empty, mut other_changes) = (0u64, 0u64);
        let mut samples = BTreeSet::new();
        sheet.cells.changes(&other.cells, |change| {
            if change.before.is_some() && change.after.is_none() && !change.visible() {
                removed_empty += 1;
                let cell = change.coordinate();
                let bucket = (
                    cell.row.saturating_sub(1) * 8 / sheet.bottom.max(1),
                    (u32::from(cell.column) - 1) / 16,
                );
                if samples.insert(bucket) {
                    let row = cell.row.saturating_sub(2).max(1);
                    let column = (bucket.1 * 16).saturating_sub(1).max(1);
                    candidates.insert(Candidate {
                        sheet: name.clone(),
                        cell: rectangle(row, column, sheet.bottom, sheet.right),
                    });
                }
            } else {
                other_changes += 1;
            }
            Ok(())
        })?;
        for row in [1, sheet.bottom.saturating_sub(7).max(1)] {
            candidates.insert(Candidate {
                sheet: name.clone(),
                cell: rectangle(row, 1, sheet.bottom, sheet.right),
            });
        }
        audits.push(json!({"sheet":name,"removed_empty_cells":removed_empty,"other_cell_changes":other_changes,
            "rows_equal":sheet.rows == other.rows,"columns_equal":sheet.columns == other.columns,
            "defaults_equal":sheet.defaults == other.defaults,"value_context_equal":before.context == after.context}));
    }
    fs::write(
        comparison.args.output.join("cell-audit.json"),
        serde_json::to_vec_pretty(&audits)?,
    )?;
    let prefixed = comparison.args.output.join("prefixed-source.xlsx");
    fs::copy(source, &prefixed)?;
    comparison.performance.path = Some(prefixed);
    comparison.telemetry.update(|state| {
        state.lane(LaneId::O).label = "Normalized baseline";
        state.lane(LaneId::SD).label = "Default shrink";
        state.lane(LaneId::CP).label = "Prefixed source";
    })?;
    comparison.start_excel_host()?;
    for lane in [LaneId::O, LaneId::SD, LaneId::CP] {
        comparison.open_workbook(lane)?;
    }
    let mut findings = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let mut captures = Vec::new();
        for lane in [LaneId::O, LaneId::SD, LaneId::CP] {
            captures.push(evidence::capture_candidate(
                comparison.excel.as_mut().expect("started host"),
                lane,
                candidate,
                comparison
                    .args
                    .output
                    .join(format!("sample-{index:03}-{}.png", lane.key())),
                &comparison.telemetry,
            )?);
        }
        findings.push(json!({"sheet":candidate.sheet,"range":candidate.cell,
            "shrink_differs":captures[0].pixels_differ(&captures[1], &mut comparison.image_comparisons)?,
            "normalization_differs":captures[0].pixels_differ(&captures[2], &mut comparison.image_comparisons)?}));
        fs::write(
            comparison.args.output.join("visual-samples.json"),
            serde_json::to_vec_pretty(&findings)?,
        )?;
    }
    for lane in [LaneId::O, LaneId::SD, LaneId::CP] {
        comparison.close_workbook(lane)?;
    }
    Ok(())
}

fn rectangle(row: u32, column: u32, bottom: u32, right: u32) -> String {
    fn letters(mut column: u32) -> String {
        let mut text = String::new();
        while column > 0 {
            column -= 1;
            text.insert(0, (b'A' + (column % 26) as u8) as char);
            column /= 26;
        }
        text
    }
    format!(
        "{}{row}:{}{}",
        letters(column),
        letters((column + 19).min(right)),
        (row + 7).min(bottom)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_ranges_stay_inside_the_sheet() {
        assert_eq!(rectangle(1, 1, 4, 3), "A1:C4");
        assert_eq!(rectangle(100, 27, 107, 53), "AA100:AT107");
    }
}
