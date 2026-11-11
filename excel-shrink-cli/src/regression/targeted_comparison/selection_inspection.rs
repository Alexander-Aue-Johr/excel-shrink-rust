//! Read-only, bounded diagnostics for the same CF dependency selector used by live comparisons.
use super::{conditional_formatting, xml_changes::Snapshot};
use anyhow::Result;
use clap::Parser;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[arg(long)]
    left: PathBuf,
    #[arg(long)]
    right: PathBuf,
    /// Maximum number of example decisions and logical regions in stdout.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(0..=100))]
    sample_limit: u16,
}

/// Inspect existing states without modifying workbooks or producing a visual verdict.
pub(in crate::regression) fn run(args: Args) -> Result<()> {
    let before = Snapshot::read(&args.left)?;
    let after = Snapshot::read(&args.right)?;
    let (changes, skipped) = before.content_changes(&after)?;
    let mut regions = BTreeSet::new();
    let decisions = conditional_formatting::select(&before, &after, &changes, &mut regions)?;
    let mut reasons = BTreeMap::new();
    for decision in &decisions {
        *reasons
            .entry((&decision.outcome, &decision.reason))
            .or_insert(0usize) += 1;
    }
    let reasons: Vec<_> = reasons
        .into_iter()
        .map(
            |((outcome, reason), count)| json!({"outcome":outcome, "reason":reason, "count":count}),
        )
        .collect();
    let limit = usize::from(args.sample_limit);
    let layout = inspect_layout(&before, &after, limit)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "left":args.left,
            "right":args.right,
            "capture_policy":super::CAPTURE_POLICY,
            "cell_or_layout_changes":changes.len(),
            "potential_cell_input_changes":changes.iter().filter(|change| change.may_change_cell_input).count(),
            "exterior_cell_changes":skipped,
            "conditional_decision_count":decisions.len(),
            "conditional_logical_region_count":regions.len(),
            "reasons":reasons,
            "decision_examples":decisions.iter().take(limit).collect::<Vec<_>>(),
            "region_examples":regions.iter().take(limit).collect::<Vec<_>>(),
            "layout":layout,
            "visual_verdict":"not_observed"
        }))?
    );
    Ok(())
}

fn inspect_layout(before: &Snapshot, after: &Snapshot, limit: usize) -> Result<serde_json::Value> {
    let mut bounds = super::content_bounds::ContentBounds::default();
    bounds.include(before);
    bounds.include(after);
    let mut regions = BTreeSet::new();
    super::layout_candidates::select(before, after, &bounds, &mut regions)?;
    let sheets: BTreeSet<_> = regions.iter().map(|region| region.sheet.as_str()).collect();
    let examples: Vec<_> = sheets
        .iter()
        .take(limit)
        .map(|name| {
            let left = &before.sheets[*name];
            let right = &after.sheets[*name];
            json!({
                "sheet":name,
                "defaults_changed":left.defaults != right.defaults,
                "unmodeled_sheet_layout_changed":left.cf_layout != right.cf_layout,
                "content_envelope":bounds.envelope(name),
                "background_tail_sampling_before":left.background_tail_sampling,
                "background_tail_sampling_after":right.background_tail_sampling,
            })
        })
        .collect();
    Ok(json!({
        "shared_layout_context_changed":before.layout_context != after.layout_context,
        "logical_region_count":regions.len(),
        "selected_sheet_count":sheets.len(),
        "sheet_examples":examples,
        "region_examples":regions.iter().take(limit).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_exposes_wide_default_layout_trigger_without_grid_expansion() -> Result<()> {
        let sheet = super::super::xml_changes::Sheet {
            visual_bounds: Some(super::super::content_bounds::Rectangle::parse("A1:XFD100")?),
            ..Default::default()
        };
        let before = Snapshot {
            sheets: [("Data".into(), sheet)].into(),
            context: "same".into(),
            layout_context: "same".into(),
            cf_context: "same".into(),
            sheet_names: None,
            defined_names: None,
        };
        let mut after = before.clone();
        after.sheets.get_mut("Data").unwrap().defaults = b"changed-default-width".to_vec();
        let result = inspect_layout(&before, &after, 1)?;
        assert_eq!(result["logical_region_count"], 1);
        assert_eq!(result["sheet_examples"][0]["defaults_changed"], true);
        assert_eq!(
            result["sheet_examples"][0]["content_envelope"]["right"],
            16384
        );
        assert_eq!(result["shared_layout_context_changed"], false);
        let limited = inspect_layout(&before, &after, 0)?;
        assert_eq!(limited["logical_region_count"], 1);
        assert_eq!(limited["sheet_examples"], json!([]));
        assert_eq!(limited["region_examples"], json!([]));
        Ok(())
    }
}
