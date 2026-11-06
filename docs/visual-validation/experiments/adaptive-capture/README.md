# Adaptive capture partition, 2026-11-02

## Question and implementation

Does cell-count tiling waste work when large ranges are hidden? Can the same
selected coverage be exported using measured image budgets instead?

`capture_layout.rs` merges adjacent internal candidate tiles without filling
holes, measures every live state through the existing Office.js workers, and
bisects only oversized visible extents. Zero-area leaves retain geometry and a
non-rendering observation. Later states can repartition and backfill earlier
states. This does not change XML/CF/rebuild dependency selection or introduce
sampling. The coverage policy remains `targeted-content-grid-v2`; the new
partition is recorded separately as `visible-geometry-v1`.

Planning uses a conservative two pixels per point plus rounding allowance,
30,000 pixels per edge and 32 million pixels total. Actual exports are checked
against the limits too. The point-to-pixel allowance is not a guarantee for every
Excel/display environment; an incompatible export fails rather than being
accepted as unchanged. Charts keep their separate export path.

## FRPP observation

The retained original `frpp_public_dataset_fy24_07022025_af7affc7a554.xlsx`
contains 307,920 rows, 304,140 hidden. All 19,248 original capture filenames were
matched to their actual cell addresses. A separate original inspection copy,
without the corpus add-in, was queried through COM:

| Measurement | Result |
| --- | --- |
| Top/Left for all original grid ranges | 31.428 seconds |
| Additional Height/Width queries | 28.484 seconds |
| Distinct Top/Left positions | 376 |
| Zero-area grid ranges | 18,872 |
| Rust adaptive planner against actual Excel geometry | 64 leaves, 710 probes, 2.815 seconds |

The last row tests the actual Rust planner through a line-oriented read-only COM
probe; it is **not** an Office.js transport benchmark or a completed multi-state
FRPP comparison. The source inspection workbook is left open and unchanged.
Native corpus geometry is still obtained through Office.js, without adding COM
geometry queries to production.

Reproduce the read-only planner observation on a separate inspection copy:

```powershell
$env:CAPTURE_GEOMETRY_WORKBOOK = 'C:\path\to\frpp-inspection.xlsx'
$env:CAPTURE_GEOMETRY_OUTPUT = "$PWD\.tmp\adaptive-review\frpp-plan.json"
cargo test -p excel-shrink-cli --lib native_frpp_visible_geometry_plan -- --ignored --nocapture
```

The ignored test intentionally uses the known FRPP extent `A1:DN307921`.
It is not a general corpus command. Local source copies, plans and capture data
are not committed.

## Native Office.js checks

A two-sheet synthetic fixture was run through the real private-desktop workflow
with Original, Default, Default rebuilt and Original rebuilt (five comparison
pairs). Inputs deliberately differ; these are transport/coverage tests, not
claims about what Excel Shrink removes.

* A 2,000-row visible sheet requires actual image-budget splitting.
* A second sheet has hidden rows and changed hidden content. Both sides retain
  zero-area observations without images or a fabricated pixel-equality verdict.
* A second run unhides a row only in Default. Its visible image is captured;
  the original remains image-free, and the pair reports `rendering_differs` and
  `geometry_differs`, with `pixels_compared: false`.

Both native workflows completed and closed their owned Excel processes. The
first selected seven regions per state, with six real images and one zero-area
observation per state. Selection/capture transport took 7.891 seconds and image
comparison 0.172 seconds on that run; startup/save/rebuild time is additional.
These are single diagnostic runs, not comparative paper-grade benchmarks.

To reproduce, use Python with `openpyxl`, create fixtures outside version control,
and invoke the existing private-desktop launcher with a separately built binary:

```powershell
python docs/visual-validation/experiments/adaptive-capture/create-fixture.py .tmp/adaptive-test/inputs
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File excel-shrink-cli/office-capture/run-targeted-comparison.ps1 -Binary "$PWD/target/release/excel-shrink-cli.exe" -Original "$PWD/.tmp/adaptive-test/inputs/original.xlsx" -Default "$PWD/.tmp/adaptive-test/inputs/default.xlsx" -Output "$PWD/.tmp/adaptive-test/result" -ObservatoryGroup "$PWD/.tmp/adaptive-test/control" -SkipCheckPerformance
```

Repeat with `--visibility-transition` when generating a new fixture directory and
use fresh result/control directories. Production commands/arguments are unchanged.

The completed-report contract is checked against both native reports. Unit tests
also cover holes, invalid geometry, a single indivisible oversized cell, long
hidden spans, sizing by another state, full cell coverage after splitting,
missing partner evidence, zero-area serialization and full-resolution context at
the far edge of adaptive rectangles.

## Result compatibility and retained limitations

Old completed results remain reusable with their original grid partition and
producer identity. They covered the same selected cell union; they are not
rewritten as adaptive results. Interrupted attempts retain partial evidence and
are resumed as new attempts. Missing positive-area evidence remains an error.

Large change vectors and raw candidate accumulation still consume memory and
analysis time. Adaptive image partitioning does not solve that separate issue.
