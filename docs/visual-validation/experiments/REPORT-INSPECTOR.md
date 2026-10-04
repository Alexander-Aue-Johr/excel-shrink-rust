# Visual validation report inspector

## Run and review

Use the existing visual-validation GUI. **Latest report** upgrades an older completed
report in place when it has no inspector assets, without opening Excel. **Rebuild report
views** rebuilds all finished, managed HTML views and review bundles. These actions do
not rerun measurements, rewrite raw results, change run timestamps or delete reports.
Active runs remain untouched. Reports stay under `docs/visual-validation/results/`.

For a new measurement, use Get picture / Strategy F / Office.js ASP.NET / batch 1
and the hospital workbook. Send the single `review.zip` printed as **REVIEW BUNDLE**.
Reports and images may contain workbook content and local paths.

## Report structure

Workbooks contain worksheets sorted by recorded worksheet index, with names preserved
exactly (including whitespace). New captures record Office worksheet position plus one;
Rust also preserves workbook order across hidden sheets. Legacy visible-only indices are
labelled explicitly. Unknown associations are not guessed.

Each worksheet shows bounds provenance before its gallery: Rust cell/merge envelope,
drawing envelope, selected Rust bounds, add-in initial request and final or retained
bounds. Pass tables show capture requests, proposed columns and edge evidence. New Rust
plans include extremal cell/merge references and counts, not cell values. The OOXML
`dimension` is recorded only as evidence; it does not become a new bound.

Images are classified as final, retained, overflow probe, calibration, failed or diagnostic.
Only the open gallery is populated, and image sources are attached near the viewport.
Offscreen sources and closed galleries are released. Browsers control actual decode cache
reclamation. The inspector supports 100%, 200% and fit-width views without re-encoding
original PNGs. Original links and detailed geometry/timing records remain available.
No CDN, web server, font download, PNG stitching or Canvas re-encoding is needed.

HTML is a view over the retained data. Missing evidence in old runs is shown as missing;
rebuilding the view cannot manufacture a historical cell witness or unrecorded event.
Bounds events are sent before captures and retained in progress snapshots and journals.

## Overflow correction

The existing thin-horizontal-rule filter now also checks the probe band across at least
two visible columns. A rule interrupted by text inside the base range can still be a
uniform thin line in the extra area. Localized glyphs and bands thicker than four pixels
are not removed by this additional test. The accepted foreground/background rule remains
an experimental heuristic, not a proof of semantic text absence.

New pass results preserve edge-hit pixels, their tile/range, estimated background and
ignored-rule counts. Growth beyond 64 additional visible columns stops with **inconclusive**
and retains the last captured area. It is never reported as proven absence of content.
Rust cell-content policy, logical bounds, raster calibration and tile safety budgets are
otherwise unchanged. No UsedRange/content rescan is added to Strategy F.

## Validation

`node docs/visual-validation/experiments/tests/report-inspector.test.cjs`

Existing safe-capture and calibration Node/Python tests remain applicable. Rust witness
unit tests are included in `range-image/rust-ooxml-plan/src/visual_plan.rs` and can be run
with its local Cargo manifest. Native Excel, Windows PowerShell/WPF and Rust build results
must still be checked on the target environment.
