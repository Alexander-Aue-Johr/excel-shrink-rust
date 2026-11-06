# Current CF fallback audit — 2026-11-02

This snapshot precedes data-bar support. The subsequent measurement in
[data-bars-20261102.json](data-bars-20261102.json) records 832 remaining structural
fallbacks and 1,698 source-clipped candidate rectangles.

The offline audit reuses the production worksheet namespace scanner, x14 adapter
and rule classifier. It scans only the 1,721 retained CF-bearing sheets from the
Data.gov inventory. It omits the disk cell index and row-layout collection, so
this is a **rule eligibility audit**, not an unchanged-input proof or a visual run.
The full audit completed in 105.56 seconds with zero errors.

## Exclusion accounting

All 493 actual unmodified prefix-input shrink reports agree:
`total=493`, `skipped=493`, `processed=0`, `preserved=0`, `failed=0`.
Every reason identifies `xl/workbook.xml` as the unsupported namespace part.
No matching workbook contains CF blocks, so these exclusions do not reduce the
CF counts below.

Corpus preparation persists `UNSUPPORTED_XML_ROOT`, the offending part and the
shrinker-report provenance. Classification makes the exclusion terminal.
The workbook index exposes the reason and marks visual identity false. Progress
counts unique workbook identities; rereading an exclusion does not count it twice.
Once the visual denominator is frozen, exclusions belong to `visual_excluded`,
not `visual_completed` or the remaining visual workload. Thus the visual-phase
`skipped` counter need not include preparation exclusions; these are distinct scopes.
Focused tests cover the reason, legacy-success sanitization, batch counts,
idempotent classification and repeated observation.

No new production exclusion policy was introduced. The two positive normalized
experimental cases remain retained separately; raw prefixed inputs still follow
the existing unsupported-namespace policy.

## Current structural fallbacks

Out of 20,935 inventoried CF blocks, **1,092 (5.22%)** retain a structural fallback,
across **215 sheets in 74 workbooks**. This includes hidden sheets present in the
inventory; the separate cost output records blocks outside the source visual plan.
The remaining 19,843 blocks have understood dependency strategies. Changed inputs,
formulas, rules, styles or other relevant context can still require captures there.
They are not automatically classified as visually equal.

| Reason | Blocks |
| --- | ---: |
| Unsupported rule semantics | 785 |
| Unsupported namespace/extension representation | 260 |
| Unsupported block attributes or child content | 44 |
| Unresolved application range | 3 |

By rule family: expression 736; dataBar 256; cellIs 25; iconSet 20; top10 16;
cellIs+containsText+duplicateValues+uniqueValues 14; colorScale+iconSet 6;
containsText 5; colorScale+dataBar 4; aboveAverage+cellIs 4;
colorScale 3; cellIs+containsErrors+containsText+duplicateValues+uniqueValues 2;
cellIs+colorScale 1. A mixed block counts once.

The three unresolved ranges are pivot color scales in
`first_6_weeks_post_fumigation_16f176f0c073.xlsx`, sheet2 (two blocks) and sheet6.
They are unknown ranges, not zero-cost regions.

## Cost interpretation

The raw inventory proxy uses 256 rows by 32 columns, sums overlapping regions,
and does not clip to used content. It is useful for spotting whole-column rules,
but misleading as an actual workload count. Nominal leaders are the two
`pcc_utilization_pivot_2017/2018` workbooks (data bars and mixed scale/bar rules)
and the Gavett lung-fiber workbook (`#DIV/0!` expression).

The additional cost audit uses the real source visual plan and production
128-row by 16-column fallback tiling. It clips ranges to each source sheet's
extent and deduplicates exactly identical candidate rectangles. It reports
excluded hidden/non-plan blocks explicitly. Different overlapping rectangles
can still overlap in pixels. Partner states may have different extents; state
pairs, native rendering times, charts and other selection reasons are outside
this cost estimate. See `current-fallback-summary.json` for the resulting ranking.


After source-plan clipping, 1,090 blocks are on planned visible sheets; two
blocks belong to hidden sheets in `forecast4320` and `forecast6920`. The fallback
footprint contains 2,549 distinct rectangles across 71 workbooks. These are
source-based selection rectangles, not measured images, jobs or elapsed time.

| Workbook | Candidate rectangles |
| --- | ---: |
| 1-s2.0-S0048969725011027-mmc2_58c946664718.xlsx | 700 |
| pcc_utilization_pivot_2018_75f08a61bac7.xlsx | 314 |
| pcc_utilization_pivot_2017_d6d443ab7354.xlsx | 313 |
| Trout_ER_Antagonism_Data_50069aa02b34.xlsx | 179 |
| real_prices_1b186fd7a7e1.xlsx | 150 |
| real_prices_3ef815db19c7.xlsx | 150 |

The first workbook's 12 fallback blocks are all data bars. The two PCC workbooks
combine data bars, mixed data-bar/color-scale extensions and `isblank` expression
content. Together those three account for 1,327 of 2,549 estimated rectangles
(52.1%). Trout's 271 expression blocks contain `#REF!`; both real_prices files
contain 132 expression blocks with `#REF!=1`. These are candidates for narrow
future dependency support, not newly accepted invariance proofs.
## Repeat without Excel

```powershell
$env:CF_INVENTORY = '<retained-inventory>'
$env:CF_AUDIT_OUTPUT = '<new-audit.json>'
cargo test --release -p excel-shrink-cli --lib audit_inventory_with_current_selector -- --ignored --nocapture

$env:CF_AUDIT_INPUT = $env:CF_AUDIT_OUTPUT
$env:CF_COST_OUTPUT = '<new-costs.json>'
cargo test --release -p excel-shrink-cli --lib rank_fallbacks_with_source_visual_extents -- --ignored --nocapture
```

Both are explicit ignored audit tests. They do not run during ordinary tests or
change production selection. Evidence remains under `.tmp`; no corpus restart,
Excel session or new full visual comparison was launched for this audit.
