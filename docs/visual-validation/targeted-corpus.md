# Targeted corpus comparison

Cell captures use the intersection of each candidate region and the content
envelope, expanded by one row and one column. The envelope is the union of
content-bearing cells in raw Original/Default, ordinary saved controls, Check
Performance and rebuilt copies. Formulas and stored strings count as content;
formatting-only cells do not extend it. This is a bounding rectangle, not a
claim that every cell inside it contains data.

Deleted real values therefore remain eligible. Deleted blank tails do not cause
hundreds of thousands of captures. `report.json.content_coverage` records each
sheet's envelope, requested count, skipped count, clipped count and unique
selected regions. Skips mean **outside the chosen coverage**, not visually equal.
Charts are exported separately, including chart sheets, even outside cell bounds.
Late rebuild candidates are backfilled in the retained pre-rebuild workbooks.

The one-cell margin is a coverage policy, not a proof that arbitrary text overflow
or formatting outside content cannot change. Reports retain this limitation.

Focused native verification:

```powershell
cargo build -p excel-shrink-cli
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/targeted-comparison/run-smoke.ps1 -Output .tmp/targeted-recalc-check -RecalcCandidate
```

Use a new output directory for each native smoke test.
