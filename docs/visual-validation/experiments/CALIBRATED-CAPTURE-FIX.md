# Measured capture scales (follow-up to consolidation v1)

The review run `20261004T194758202Z-f8cfc4b7` produced 113 structurally valid PNGs,
all matching `2 * boundary-point-extent + 1` on both axes. The fixed 4/3 model
rejected every PNG and recursively split otherwise usable captures. No final tile
was accepted. This is a capture-validation defect, not evidence of 113 Excel crashes.

## Same workflow; no extra experiment

Re-run **Get picture / F / Office.js ASP.NET / batch 1 / hospital workbook**.
The run directory, report buttons and `review.zip` workflow are unchanged.

Before a sheet's first tiled capture, the capture engine measures three small
unaltered ranges and a fourth validation control. Each PNG is persisted. The
controls vary both dimensions, fit x and y independently and validate the existing
one-pixel image-edge convention. A bad or capped PNG cannot calibrate the model.
No fixed factor of 2, browser-DPR multiplier or 96-DPI result from a previous run
is used for final planning. Controls may read cells just outside the logical range;
they do not change bounds, content, formatting, zoom, selection or activation.

Small bootstrap controls use a bounded conservative scale envelope up to 8 px/pt,
8192 px/edge and four million pixels. These are engineering assumptions, not
universal API guarantees. An unsupported environment stops with a diagnostic.
Four controls add measured overhead per sheet; calibration time overlaps its
geometry/render/persistence/telemetry components and must not be summed again.

## Budgets and failure classes

Final tiles permit at most 30000 px per edge (below the observed 32766 cap) **and**
16 million pixels in total. The higher edge budget preserves full horizontal
context for wide, shallow overflow strips at 2 px/pt; the area limit is not raised.
The actual PNG is validated before any Canvas decode. Capture and Canvas use the
same budgets. Raw PNGs, per-tile `raster_profile`, rectangles and offsets are retained.

A dimension at the observed cap or an actual budget excess can trigger subdivision.
A normal native failure can trigger a small health control and smaller regions.
A mismatch against a calibrated scale stops the workbook; it does not recursively
split to a single cell or silently mix different raster profiles. Retry by starting
another run with a stable display environment. This does not prove the Windows DPI
setting caused the observed factor: effective DPI is arithmetic, not a measurement
of the window/monitor. Browser DPR, screen dimensions, Office version and calibration
samples are logged for diagnosis.

## Reports

The central summary includes accepted tiles, completed/failed/inconclusive sheets,
raw tile attempts, calibration images, terminal sheet errors and grouped attempt
errors. Recovered attempts alone do not turn a successful sheet into a failure.
A run with all sheets failed is `failed`, not merely `partial` with empty `errors`.
A returned workbook result is not labelled successful just because it was received.

The patch does not rebuild previous reports, remove history, modify production
regression code or introduce a new experiment. Actual Excel/WPF/Windows rendering
still needs a run on the user's machine. Test mocks cover protocol and geometry,
not native Office rendering.
