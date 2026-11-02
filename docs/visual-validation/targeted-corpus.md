# Targeted corpus comparison

Cell captures use the intersection of each candidate region and the content
envelope, expanded by one row and one column. The envelope is the union of
content-bearing cells in raw Original/Default, ordinary saved controls, Check
Performance and rebuilt copies. Formulas and stored strings count as content;
formatting-only cells do not extend it. This is a bounding rectangle, not a
claim that every cell inside it contains data.

Changed cells and candidate rectangles select a common 128-row × 16-column grid,
reusing the conditional-formatting grid size. Each tile is captured once per state
and clipped to the content envelope. Neighboring tiles remain separate. This
avoids one screenshot per changed cell without limiting candidate or tile counts.
`capture_policy: targeted-content-grid-v2` records this coverage policy explicitly.

Deleted real values therefore remain eligible. Deleted blank tails do not cause
hundreds of thousands of captures. `report.json.content_coverage` records each
sheet's envelope, requested count, skipped count, clipped count and unique
selected regions. Skips mean **outside the chosen coverage**, not visually equal.
Exterior empty-cell XML changes are counted before allocating candidate objects;
each transition records `skipped_outside_content_cell_changes` per sheet. These
counts are per transition, so they must not be summed as unique workbook cells.
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

## Corpus commands

The WPF launcher now selects this strategy by default, including when loading an
older machine configuration. Legacy screening flags in those configurations are
ignored while `settings.targeted_comparison` is true. Shrinking prepares Default
only. Check Performance is generated on demand from the saved Original control.
No legacy full visual pass follows the targeted comparison.

```powershell
cargo build --release -p excel-shrink-cli
.\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/targeted-study --run-corpus Data.gov --resident-capture --targeted-comparison --control-room --comparison-workers 4 --order size-asc
```

Use your machine's existing request/configuration paths; keep its archive binding
and corpus IDs. Add `--targeted-skip-check-performance` for Original/Default only.
`--size-part 1/4` through `4/4` and `--order size-desc` retain their existing meaning.
Do not combine targeted mode with `--preparation-hash-profile`, `--aggressive-first`
or `--full-scan-deferred`. The workflow uses four retained states without Check
Performance and six with it, so concurrent groups multiply that process count.

Resume the **same output root and arguments** to reuse completed targeted results.
Old visual/hash results are not accepted as completion of this policy. A completed
targeted result remains explicitly limited coverage (`checks_complete: false`).
Namespace exclusions keep their existing reasons. When Check Performance is
requested, an unchanged Default does not skip that independent optimizer lane.
Proven nonvisual Default-only exclusions remain available when CP is omitted.

Existing prepared bundles can be reused with `--reuse-prepared-from <full executable
SHA-256>` after reviewing shrink compatibility. Keep the associated records and
source catalogs; copying a ZIP alone is insufficient. Use `--compare-only` when
all required preparations are already present. Never reuse old comparison verdicts
as benchmark results. The local slowest-100 corpus and its baseline remain outside Git.

Durable evidence contains `portable-report.json`, cell PNGs and converted chart PNGs.
These follow records through the existing `--export-results` / `--import-results`
verified transfer. Machine/run attribution remains separate for preparation and
comparison. Completed temporary XLSX states are removed to bound disk use; raw
Original/Default remain recoverable from preparation bundles. Native smoke tests
retain their states for inspection.
