# Pivot cellIs investigation — 2026-11-02

The eight remaining pivot-tagged cellIs blocks stay in capture fallback. Their
new explicit decision reason is `pivot-formatting-dependencies-not-modeled`.
Capture selection and coverage are unchanged; this is not a new invariance proof.

## Evidence from the two original workbooks

| Source | Worksheet / stored sqref | Operator / threshold | Related pivot definition |
| --- | --- | --- | --- |
| SAM DCF Results (04204) | sheet1, BW6:CC14 (two blocks) | greaterThan 1 / 100 | pivotTable1.xml, location BV4:CC14 |
| combined_monthly_summary (06160) | sheet11, B7:S26 | greaterThan 0 | pivotTable9.xml, location A4:AK26 |
| combined_monthly_summary (06160) | sheet10, B7:S11 (two blocks) | greaterThan 0 | pivotTable8.xml, location A4:AK11 |
| combined_monthly_summary (06160) | sheet9, B7:S17 (two blocks) | greaterThan 0 | pivotTable7.xml, location A4:AK17 |
| combined_monthly_summary (06160) | sheet8, B7:S20 | greaterThan 0 | pivotTable6.xml, location A4:AK20 |

Every inspected rule priority maps to additional `conditionalFormat` and
`pivotAreas` metadata in the related pivot definition. These include field,
selected-element and layout attributes. Thus `pivot="1"` is not merely an unused
worksheet attribute. The current CF context reader does not compare these pivot
parts or resolve their application membership. Equal cached worksheet cells and
constant operands alone do not supply that missing proof.

A safe future exclusion would need to account for the relevant pivot definition,
its dependencies and effective application membership, including behavior across
saving/rebuild or refresh. This investigation does not claim that a refresh
actually changed these files or that their current sqref is wrong.

The eight cellIs blocks occupy nine small candidate rectangles under the existing
source-based fallback tiling (one in SAM, two on each of the four monthly-summary
sheets). This is an estimate, not rendering time. Keeping these few captures costs
less implementation complexity than adding an incompletely validated pivot
resolver now. The external workbook-reference fallback is also unchanged.

## Reproduce

```powershell
python docs/visual-validation/experiments/cf-inventory/inspect_pivot_cell_is.py <retained-inventory> <new-output.json>
```

The script reads only cached CF nodes and related package metadata. It follows
worksheet relationships, checks that all eight rule priorities have a unique
pivot match, and retains pivot-area XML plus source-part SHA-256 fingerprints.
It never opens Excel, edits a workbook or resumes the corpus. Local evidence:
`.tmp/pivot-cell-is-investigation-final-20261102.json`.

The focused Rust control verifies that pivot flags keep capture selection for
identical input values and now explain why. The production fallback remains
conservative for unmodeled flag values; no additional pivot variant is accepted.
