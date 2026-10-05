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


## 2026-10-05 review: budget consistency and dense candidates

Status: implementation defects identified; corrected implementation needs a native rerun.
Evidence: review(3).zip (run 20261004T235041658Z-b9b8f410), review(4).zip
(run 20261004T235322225Z-3293bcdc), review(5).zip
(run 20261004T235617404Z-38fdb10f). All three selected **B only**, not Compare A and B.
They all used Strategy F, the ASP.NET host, batch 1 and budget-aware packing.

| Selected MP | Initial KHV tiles | Actual KHV retained tiles | All tile attempts | Accepted tiles | Canvas-budget rejections | Runner workbook seconds |
|---:|---:|---:|---:|---:|---:|---:|
| 16 | 47 | 47 | 64 | 64 | 0 | 60.68 |
| 32 | 24 | 70 | 137 | 87 | 50 | 112.60 |
| 64 | 12 | 78 | 167 | 95 | 72 | 160.84 |

Nine sheets completed and two (KHV/RHV) were inconclusive in each run. The root
wrapper reported failed because the required capture validation was incomplete.
No native render exception was recorded in these tile attempts. `CANVAS_BUDGET`
was thrown by our own decoder *after* Excel had returned and the host saved the PNG.
Do not interpret these timings as evidence that Excel is intrinsically slower at
32/64 MP or that the selective method beats/loses to A.

Defects and corrections:

- The planner used the selected 16/32/64 MP budget while the decoder still read
  `SafeCapture.DEFAULTS.max_pixels` (16 MP). The decoder now receives per-sheet limits,
  validates the PNG header before allocating a browser image/canvas, and frees failed
  allocations. 32/64 MP remain opt-in experiments, not proven memory-safe defaults.
- Any nonfatal exception, including analysis errors after a saved PNG, used to trigger
  a native recovery screenshot and recursive subdivision. Recovery is now limited to
  render/payload failure and explicit capture-size rejection. Analysis and persistence
  failures retain the saved image, record the failure stage and propagate, without
  recapturing it. Failed-analysis time is no longer dropped from timing counters.
- Fixed groups of 16 candidate rows were not a pixel budget. The first KHV group was
  15440 x 288 = 4,446,720 pixels, above the independent 4-MP evidence budget. Groups
  are now packed from measured candidate-row boundaries *before* native evidence captures or writes. At most 16
  blocker cells can be active in one group. A single oversized row remains inconclusive.
- The old 128-candidate limit was a whole-tile mapping limit, not a simultaneous-write
  safety limit. Mapping is bounded at 2048 candidates, while each write retains the
  smaller group limit. Candidate and group plans are persisted even when no intervention
  can proceed. Counters distinguish planned, attempted and confirmed cell writes.

There is also a substantive cost observation: on the first KHV tile the raw detector
selects all 1034 interior pixel rows. Seven vertical gridlines in the probe region are
sufficient to trigger this. This is not an isolated two-row signal. The raw scanner
has deliberately NOT been weakened in this correction: ignoring grid colors or
silently substituting A's line heuristic would change the meaning of the B audit.
A dense verification scan can still be costly after the budget defects are fixed.
Reports show candidate density, measured evidence groups and unresolved reasons.

Seventy-three completed interventions were independently decoded and checked across
the three archives: every restored image was pixel-identical to its before image,
and every full-image changed-pixel count matched the recorded count. There were
27 blocker-sensitive outcomes (nine repeated across three runs), 46 unchanged outcomes,
and 732 confirmed cell writes in these completed checks. These are not 73 independent
workbook cases and do not establish a general text-versus-decoration classifier.
In particular, the KHV 16-MP checks never wrote any blockers: they stopped at preflight.

Next native run: F / ASP.NET / batch 1 / Compare A and B / 16 MP / budget-aware rows,
on the hospital workbook. Do not run the full budget sweep before verifying this
correction. Keep the production minimum-width policy unchanged. Source control tests
and mocks do not substitute for an Excel/Windows run.
