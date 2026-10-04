# Consolidated capture and reports (v1)

This is an engineering safety patch for the experiments, not a change to the Rust
production regression pipeline or an assertion that Office.js is now the fastest backend.

## Run the failing hospital case

Use the existing visual-validation GUI: Get picture, Strategy F, Office.js ASP.NET,
batch size 1, krankenhausverzeichnis-3500100227005.xlsx. No new fixture experiment is required.
The original Rust content/drawing/hidden-tail plan is still the source of logical bounds.

## One place to look

All supported experiment entry scripts use `docs/visual-validation/results/`.
Open `index.html` or use the GUI **Reports** / **Latest report** buttons.
Every run is `runs/<UTC-time-id>/` with `run.json`, `result.json`, `summary.txt`,
`report.html`, `console.log`, step results/progress/events, images and `review.zip`.
`result.json` is a small common envelope referring to original backend JSONs; it does not
replace their schema or discard raw measurements. `latest.json` points to the newest run.
The paths are printed before native work and again from the outermost finally block.

Handled failures preserve a ZIP. A process termination/power failure cannot execute finally;
use **Recover interrupted runs** afterwards. A started event without a corresponding
completed event is retained as incomplete, never converted to a successful measurement.
Disk/permission failures can still prevent report creation; the original error is not hidden.

**Prune history (keep 10)** previews/confirms deletion and keeps the last 10 finished managed
runs. Active runs, unknown directories, symlinks, inputs, golden baselines and build caches
are not removed. Old pre-consolidation report directories are left untouched; new runs no
longer require browsing those locations. No historical result is deleted by applying the patch.

## Capture contract

Normal Office.js range-image capture uses **tiles-with-valid-rect-v1**. Default budgets are
30,000 pixels per edge AND 16,000,000 pixels per tile, one native getImage request at a time.
Those are conservative engineering budgets, not documented PNG/Excel maxima. Timings record
geometry, native rendering, persistence and analysis separately; requested batch size is no
longer native image concurrency. Do not compare old monolithic timings as like-for-like runs.

The planner reads top/left coordinates of rectangle boundaries, splitting oversized rectangles
breadth-first in batched geometry reads. It does not scan all cells or replace logical bounds.
The first cell AFTER each edge is used; at the worksheet's last row/column an explicit terminal
cell-size fallback is recorded. Each returned PNG is checked against fresh boundary coordinates
and the per-worksheet calibrated raster profile. Geometry changes are reported rather than silently normalised.

Every raw tile is persisted before browser overflow analysis. F analyses bounded full-width row
strips and combines the maximum overflow column across strips. The existing background/rule
heuristic remains experimental. Its column edges now use measured left-coordinate differences,
not a global proportional conversion of XML character widths. No giant Canvas is allocated for
cropping. `valid_rect_px`, `sheet_offset_px` and `logical_address` identify the final view inside
raw PNGs, which may retain probe margins. Do not compare raw probe margins as final content.
No giant combined PNG is created. Seam/pixel equivalence still needs validation before production.

If horizontal splitting is necessary, potential far-left text sources are no longer guaranteed
in each capture. F retains the extra area and reports **inconclusive** rather than pretending
exact overflow was measured. A/B/C final captures use the same safety layer. D/E final images
also use it, but their legacy XOR probe stage is only budget-guarded, not rewritten as tiled XOR.
An oversized XOR probe fails explicitly before mutation. The VBA screenshot baseline is likewise
budget-guarded rather than silently changed into another algorithm. Native drawing benchmarks
retain their methods and share the report wrapper and preserved outputs.

On a recoverable native error, a small control is tested and the failed tile is divided,
not retried at its original size. Budgets bound recursion/counts. After a timeout no further
Office calls are queued: a JS timeout cannot cancel an already running native request.
Incomplete sheets and artifacts remain in progress files. Healthy-session sheet failures do
not discard other completed sheets. Reports distinguish complete/partial/failed/interrupted.

## Tests

`node docs/visual-validation/experiments/tests/safe-capture.test.cjs`
Existing calibration/row-height Node and Python tests remain applicable.
These do not replace testing actual Excel, WebView, COM, PowerShell/WPF and native host builds.

The Oct 4 log motivating this patch shows a valid 15440 x 32766 getImage PNG followed by
zero Base64 characters after browser cropping. Canvas.toDataURL can return `data:,`:
https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/toDataURL
This is separate from Excel's observed 32766 clipping and from transient getImage failures.

See [report inspector and bounds provenance](REPORT-INSPECTOR.md) for lazy screenshots, English UI labels, immutable bounds stages and bounded overflow expansion.
