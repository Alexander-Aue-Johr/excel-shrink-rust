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
            "visual_verdict":"not_observed"
        }))?
    );
    Ok(())
}
