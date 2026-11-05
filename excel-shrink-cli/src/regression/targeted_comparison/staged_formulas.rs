//! Compact, optional explanations from saved states; separate from verdicts.
use super::Workflow;
use anyhow::{Context, Result};

const MAX_DIAGNOSTIC_REGIONS: usize = 32;
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

impl Workflow<'_, '_> {
    /// Preserve small formula explanations before disposable saved states are
    /// removed. Diagnostic budgets or parsing failures never revise a verdict.
    pub(super) fn record_formula_diagnostics(&mut self) {
        use crate::regression::formula_trace;
        let mut regions = Vec::new();
        let mut omitted = 0;
        let mut bytes = 0;
        for (pair_index, pair) in self.comparison.report.comparisons.iter().enumerate() {
            let different: Vec<_> = pair
                .findings
                .iter()
                .enumerate()
                .filter(|(_, finding)| {
                    finding.pixels_differ || finding.rendering_differs
                })
                .collect();
            if different.is_empty() {
                continue;
            }
            if regions.len() >= MAX_DIAGNOSTIC_REGIONS || bytes >= MAX_DIAGNOSTIC_BYTES {
                omitted += different.len();
                continue;
            }
            let books = (|| -> Result<_> {
                let left = self
                    .states
                    .get(&pair.left)
                    .context("Compared left state is not retained")?;
                let right = self
                    .states
                    .get(&pair.right)
                    .context("Compared right state is not retained")?;
                Ok((
                    formula_trace::read(&left.path)?,
                    formula_trace::read(&right.path)?,
                ))
            })();
            for (finding_index, finding) in different {
                if regions.len() >= MAX_DIAGNOSTIC_REGIONS || bytes >= MAX_DIAGNOSTIC_BYTES {
                    omitted += 1;
                    continue;
                }
                let diagnosis = match &books {
                    Ok((left, right)) => formula_trace::compare(left, right, &finding.sheet, &finding.cell),
                    Err(error) => Err(anyhow::anyhow!("{error:#}")),
                }.unwrap_or_else(|error| serde_json::json!({"evidence":"unavailable","reason":format!("{error:#}")}));
                let entry = serde_json::json!({
                    "pair_index": pair_index,
                    "finding_index": finding_index,
                    "kind": "findings",
                    "diagnosis": diagnosis,
                });
                let size = serde_json::to_vec(&entry).map_or(usize::MAX, |json| json.len());
                if size > MAX_DIAGNOSTIC_BYTES - bytes {
                    omitted += 1;
                    continue;
                }
                bytes += size;
                regions.push(entry);
            }
        }
        self.comparison.report.formula_diagnostics = serde_json::json!({"version":1,"regions":regions,"omitted_regions":omitted,
            "note":"Optional dependency explanations; no visual verdict or coverage was changed."});
    }
}
