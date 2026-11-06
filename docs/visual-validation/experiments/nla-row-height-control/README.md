# NLA automatic-height control

## Question

The Data.gov comparison of `NLA water isotope data-2007-2017_5289b0294534.xlsx`
reported heights of 15.75 versus 15 points for rows 6–179 of
`2007 NLA QA Data`. Both states were rebuilt in their resident processes.
The resulting one-pixel-per-row displacement produces a large image difference.

## Counterfactuals observed on 2026-11-06

Experiments used disposable copies, private desktops, Default only and runner
build `77bbca5`. One regular group slot was reserved during these probes, then
the regular target was restored to five groups. Corpus bundles and receipts
were preserved.

1. Restore only the original column definitions on `xl/worksheets/sheet2.xml`
   in Default. The same 15.75-to-15-point height difference remains.
2. Restore the complete original `sheet2.xml` in Default. The difference remains.
   The saved rebuilt states have byte-identical `sheet2.xml`, `styles.xml` and
   `theme/theme1.xml`, despite their different measured effective heights.

Each native probe completed with seven selected regions per state, fourteen
successful captures and confirmed closure of its owned Excel processes.
The initial invocation without an Observatory directory failed its entry guard
before workbook observation. The successful probes used an existing directory.

Offline replay with the current shrink binary also produced the same
uncompressed package parts as the retained Default bundle. An obsolete prepared
bundle therefore does not explain this particular difference.

## Interpretation and remaining check

Restoring columns alone does not fix this case. Restoring the entire worksheet
also does not fix it. The observation cannot be attributed solely to a local
worksheet edit. Other workbook parts still differ in these counterfactuals.
Excel session state, workbook-wide effects and the observation workflow remain
possible causes. A same-package, two-session height control is still needed.

The production difference remains unclassified. No height normalization,
automatic pass or shrink-rule change follows from these probes.

Local evidence is retained under
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp\nla-column-height-proof`.
`result-native` contains the column control and `result-sheet2-control` contains
the full-worksheet control. Their reports, telemetry, native height checks,
saved workbooks and PNGs are retained for follow-up.
