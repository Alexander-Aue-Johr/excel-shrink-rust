# Selective overflow verification and capture-budget comparison

## Status and scope

Engineering experiment, not a change to the production visual-regression contract.
A is the current Strategy F pixel heuristic (including the probe-local thin-line filter).
B sees candidate pixels before that filter and verifies only their worksheet rows with
one whitespace blocker per row in the first visible column after the Rust base bound.
The existing A/B/C strategy letters still describe bounds planning; A and B here label
**verification arms within Strategy F**, not alternative bounds strategies.

The supplied review dated 2026-10-04 already contains eleven complete worksheets and
81 accepted tile images. It must not be described as a native render failure. Its final
runner status was affected by publication timing. Earlier repeated line-triggered
extensions and the empty Canvas crop remain historical incidents; A is not deliberately
regressed to reproduce them.

## Experiment map

| Investigation | Purpose | Decision/status |
| --- | --- | --- |
| Drawing bounds | Compare VBA, C# in-process add-ins, Office.js and external COM | Timing/reliability comparison; no new winner selected here |
| Get picture A–C | Compare bounds discovery and capture cost | Calibrated safe tiling |
| Get picture D/E | Legacy horizontal/vertical before/blocked XOR | Current implementation already writes one boundary column/row, but across all bounded rows/columns |
| Get picture F / heuristic | Rust bounds plus pixel evidence | Current comparison arm A |
| Get picture F / selective | Candidate-row intervention | Experimental arm B; validate using real Excel results |
| Height plateau / markers | Detect native clipping/saturation | Explicit diagnostics; not a recommended production image size |
| Units / row heights | XML geometry, raster scale, rounding and custom height | Prior observations; per-sheet raster calibration retained |

Production still uses native Rust cell/content bounds, a reasonable minimum screenshot
width rather than mandatory exact overflow, and persistent screenshots. This patch does
not introduce an Excel-side content-bounds scan or modify the core regression tests.

## Sparse intervention

1. Capture and persist the original tile. Detect candidate pixel-y bands only in its
   extra-column probe region. Map them to worksheet rows using measured `top` positions.
   Pixel scanlines are not worksheet row numbers; a boundary stroke can require two rows.
2. Bound work to 128 candidate rows / 128 bands per tile; group at most 16 adjacent rows.
   Do not allocate a full-column values array. Guard each actual blocker cell and its
   table/pivot neighborhood. Require an ordinary empty, non-merged, non-spill cell and an
   unprotected sheet. Rotated/vertically merged source contexts remain inconclusive.
3. Recapture a small full-width row group before writing. Its probe pixels must match
   the corresponding original-tile pixels. Full left context is retained. No comparison
   of different cropped ranges is accepted as evidence.
4. Journal the intended cells before queueing a values-only single-space write. Verify
   the write, capture the same range, restore only the verified-empty cells, verify
   emptiness and capture again. Restored pixels must equal the before pixels exactly.
   Every raw evidence image is persisted. Geometry and raster dimensions must remain stable.
5. Compare decoded RGBA pixels, and persist the XOR PNG. A zero difference supports
   **no blocker-sensitive text overflow in those verified probe rows**, not absence of
   arbitrary visible layout. Unchanged rules/fills do not force rightward expansion.
   Differences outside the verified probe region or unstable restoration are inconclusive.
   If changed pixels reach the right edge, the range may widen within the existing budget.
6. Never queue restoration or other Office calls after a native timeout (the native
   operation cannot be cancelled). Report restoration as unknown and discard the runner's
   disposable working copy. Original source workbooks are never saved or modified.

These are safeguards, not proof for every Excel rendering feature. Conditional formatting,
volatile calculations, unsupported objects, geometry changes and API limitations may
produce an inconclusive result. A skipped check is never converted to successful no-overflow.

## Capture dimensions and tile packing

Both edge length and total pixel area are limited. Standard = 16 MP; optional 32 / 64 MP
budgets are explicit experiments, not known-safe or optimal settings. Maximum edge remains
30,000 px, below the observed 32,766 px clipping plateau. A 15,440 × 30,000 image would
contain 463.2 million pixels and require about 1.85 GB for **one** RGBA buffer, before
renderer/decoder/Canvas copies. PNG file compression does not remove this working-memory cost.

`Budget-aware row boundaries` packs the largest measured fitting next row boundary instead
of repeatedly bisecting into power-of-two strips. The prior `Balanced subdivision` remains
selectable. Each plan records allowed strip height, area utilization, geometry, calibration
and the limiting budget. Larger tiles reduce call count, but are not guaranteed faster.
Candidate verification uses an independent four-million-pixel budget.

## Run and review

Use the existing GUI: Get picture → Strategy F → Office.js ASP.NET → Batch 1.
Choose `Compare A and B (fresh workbook per arm)`, `16 MP - Standard`, and
`Budget-aware row boundaries`. Each arm starts from an independent source-workbook copy.
The order is A then B; a single pair does not control warm-cache/order effects or establish
statistical significance. Both arms use the same tile budget and packing method.

CLI from Windows PowerShell, repository root:

```powershell
.\docs\visual-validation\experiments\range-image\run.ps1 `
  -Strategy F -Implementation officejs-dotnet -BatchSize 1 `
  -Workbook krankenhausverzeichnis-3500100227005.xlsx `
  -OverflowMode compare -TileMegapixels 16 -TilePacking budget -NonInteractive
```

The existing unified result directory and review.zip remain the only sharing workflow.
The report separates original/final tiles from verification images and shows cell-write,
restore, native-render and overall analysis timings. Analysis includes the intervention;
timing columns overlap and must not be summed. The history is a read-only offline catalog.
Pruning/recovery stay explicit launcher actions. No old reports are automatically deleted.

## Local tests

```text
node docs/visual-validation/experiments/tests/safe-capture.test.cjs
node docs/visual-validation/experiments/tests/tile-packing.test.cjs
node docs/visual-validation/experiments/tests/selective-overflow.test.cjs
node docs/visual-validation/experiments/tests/report-inspector.test.cjs
node docs/visual-validation/experiments/tests/history.test.cjs
```

Node native/render tests are protocol mocks, not actual Excel tests. PowerShell/WPF,
Excel and host integration must be verified on Windows. No real speedup is asserted by
these tests. Successful image capture does not itself assert a baseline image match.
