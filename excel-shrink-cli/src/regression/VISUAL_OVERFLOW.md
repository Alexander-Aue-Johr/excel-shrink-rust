## Combined capture and overflow (current behavior, cache version 14)

The active screenshot path no longer calls the separate `Test-OverflowStrip`
loop. Rust records `<cols>` width/hidden intervals and `defaultColWidth` in the
existing value-summary pass and cache. Missing defaultColWidth currently uses
8.43 as a proportional weight; no font/pixel conversion is attempted.

For each pair, capture the base bounds plus 1 visible column. If the last
inspectable extra column contains pixels, retry with +8, +16, +32, ... visible
columns relative to the original base. Hidden columns and width=0 do not consume
the budget. A column visible on either side counts for the pair. For example,
after base C, D visible and E:CZ hidden (100 columns), +8 ends at DG (column 111),
108 indices past C. Endpoints never exceed XFD; if the entire suffix is hidden,
or the base already reaches XFD, skip overflow analysis and capture normally.

PowerShell publishes each completed full-image/tile manifest to the existing
Rust parent using atomic request/response JSON files. Rust services requests at
10ms intervals; progress-log polling remains at 500ms. No Rust subprocess is
spawned for an image. Pixel analysis checks all extra columns over all tiles and
both workbook versions. Widths are assigned proportionally to the bitmap width;
0/1-pixel columns are ignored, and the outer image border is excluded. A filled
last visible extra column requests a larger capture unless the grid has ended.
Trailing blank columns are removed by cropping the saved PNG, never by another
CopyPicture. Both images keep the maximum content column found on either side.
The minimum retained range is always the original Rust/dimension/drawing bounds.
This scan does not search for disconnected content behind an entirely blank
first extra column or for content below the base last row.

Full compatible cache pairs still skip captures. Individual side caches can be
reused if they exactly cover the current candidate range; a narrower cropped
cache cannot prove the added guard column blank and may need recapture. Large
images retain the existing recursive tile splitting. Cropped PNGs and tile
metadata are updated together; input/cache PNGs are never overwritten.

Timing: `excel.overflow.roundtrip`, `screenshot.combined_overflow`,
`rust_analysis_and_crop_ms`, existing `excel.overflow.copy`/`excel.png_save`, and
`overflow combined finished ... attempts=`. One normal capture is per workbook
version and image tile, not one capture for the whole original/shrunk pair.
The calibration screenshot is still once per Excel capture process.

Validated in WSL/Windows: 6 combined-overflow tests including real PowerShell IPC
without Excel, 29 value-comparison tests, 59 automation/cache tests, PowerShell
orchestration and existing overflow tests. No real-workbook speed measurement
was performed for this change. Commands:

```bash
cargo test -p excel-shrink-cli combined_overflow -- --include-ignored
cargo test -p excel-shrink-cli regression::xlsx_value_compare
cargo test -p excel-shrink-cli regression::excel_automation
```

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\excel-shrink-cli\src\regression\test_combined_overflow.ps1
```

The sections below document earlier steps; the combined path above supersedes
the separate right-column capture loop.

# Rust bounds and measured Excel overflow

Implementation handoff, 2026-09-27.

## Data flow

1. `value_views.rs` already derives maximum content row/column while comparing borrowed XML cell views. Values, formulas and merged ranges contribute; formatting-only cells do not. This scan and its persistent digest cache are reused. No separate decoded DOM, reverse-cell heuristic or Excel `Cells.Find` was added. A simple reverse search would not establish the maximum column on other rows and would need extra validation for omitted/unsorted coordinates, inline/shared strings and empty formulas.
2. Before opening either input in Excel, `visual_drawing_bounds.rs` follows worksheet relationships, inflates drawing XML and reads it with `xml_views::Reader`. Drawing marker maxima are combined with the existing data bounds, across both original and shrunk inputs. The complete transfer plan is written as `screenshot-rust-bounds.json`.
3. Rust also parses offsets and extents for two-cell, one-cell and absolute anchors. Physical coordinates remain in points in the transfer plan: Excel maps them to its actual row/column geometry, without parsing XML or inspecting cell values. This avoids guessing font-dependent column widths in Rust. Rotations, effects, explicit line thickness, fixed editing modes, VML/OLE/control relationships and unknown anchors request the existing COM shape-envelope fallback. Unresolved geometry fails the sheet rather than silently cutting it off. Ordinary two-cell drawings need no per-shape COM enumeration.
4. `excel_visual_overflow.ps1` probes one column at a time immediately beyond that combined boundary, from row 1 through the final data/drawing row. It scans every vertical segment, even after a positive segment. A completely empty visible column stops extension; a positive column advances it. Hidden columns cannot establish a blank edge. Each workbook is checked, and the capture uses the union of the resulting bounds.
5. Screenshot capture and comparison continue on those bounds. Existing guarded cache reuse remains; cache version 11 invalidates pre-change screenshots. A missing Rust limit or unresolved overflow is a visible screenshot error, not a UsedRange fallback.

## Height planning

A scratch-sheet bitmap calibrates pixels/point. First, `Range.Height` checks whether the whole height already fits; this includes ranges with many hidden rows but only a page of visible content. Otherwise a 1600-row initial guess is checked; oversized guesses decrease until safe. At most two proportional upward corrections are allowed, only if the estimated saved copies over the remaining height outweigh the next height-query cost. Target: at most 32000 pixels on an axis. The safe row count is reused, while every subsequent image is checked against its actual range dimensions; varying row heights therefore still trigger safe splitting. Copy errors recursively split both halves; an unrecoverable single-row failure never counts as blank.

The same dimension verification is used for final comparison screenshots. Predictably oversized images split before CopyPicture. Actual bitmap clipping is also checked after capture. No zoom assignment and no fixed pre-copy pause remain in the active range capture. Clipboard reads start immediately and poll at 10 ms only as needed, retaining owner/sequence verification.

The pixel check treats a difference from the calibrated blank background as visible content (including fills/borders). It is not OCR. The outer one-pixel bitmap boundary is ignored. A blank adjacent strip is a stopping heuristic; drawing bounds are included first so detached graphics are not missed merely because a gap precedes them.

## Instrumentation

Rust's existing timing JSONL includes `values.inflate_sheet`, `values.cells_and_bounds`, `screenshot.drawing_bounds` and `screenshot.drawing_xml`. Cached value summaries skip inflation/scanning as before.

`screenshot-progress.log` includes `excel.overflow.calibration`, `drawing_geometry`, `height_query`, `plan`, `range`, `copy`, `pixels`, `com_release` and `total`, plus each tested column and split. Existing open/clipboard/PNG timings remain. Nested stages overlap: do not add all phase totals together. Bounds output includes source attribution (`drawing` when a marker extends an axis).

### Hidden-tail performance fix

The original overflow loop visited hidden columns one by one through XFD. This caused thousands of COM queries and individual release log lines for A4-like worksheets, notably the 215-sheet `verkehrsunfaelle-zeitreihen-xlsx-5462403.xlsx` workbook. `Find-NextVisibleOverflowColumn` now checks the immediate column and the entire remaining suffix width. A fully hidden suffix ends the probe after two width queries and needs no image. If any visible column remains, a binary search finds the first one in at most 16 width queries in total, without overlooking a visible island beyond hidden columns. Zero aggregate width is the criterion, not `Hidden` on mixed ranges, and no assumption about print areas or A4 geometry is made.

`excel.overflow.visible_column` records the first/next column and width-query count. `com_release` is aggregated per `Expand-RustVisualCaptureRange` invocation (`count=... aggregate=sheet`), including failure paths; there is no log line for each temporary COM object anymore. A physical full-height fit uses one image even if many underlying rows are hidden.

The PowerShell helper is embedded in the CLI via `include_str!`. Rebuild/restart the CLI to pick up this fix. An already running PowerShell process keeps its loaded functions. Existing screenshot-cache validity is unchanged: this fix reduces equivalent visibility queries and logging, not the required image bounds.

## Validation and WSL follow-up

Windows checks: PowerShell syntax; `test_visual_overflow.ps1` (blank/edge pixels, safe height planning, both halves despite early positive, unrecoverable error); an actual Excel smoke workbook with a detached red rectangle and bottom-row overflow. The smoke test passes explicit JSON limits matching the Rust transfer schema; it does not execute the Rust parser.

Rust was deliberately not built or tested on Windows. Run in WSL:

```sh
cargo check -p excel-shrink-cli
cargo test -p excel-shrink-cli drawing_bounds_tests
cargo test -p excel-shrink-cli regression::xlsx_value_compare
cargo test -p excel-shrink-cli regression::excel_automation
```

Then run the repository's visual regression command against the corpus. Pay particular attention to GENESIS-Online, point anchors, VML/rotated shapes, original-cache-only capture, hidden columns and unequal row heights. All existing staged changes were preserved. The legacy C# text/OOXML planner files remain for reference but are no longer initialized by the active screenshot path.

## XML-only drawing bounds
Drawing capture now uses Rust XML anchor bounds without COM Shape enumeration. Excel is still queried for cell positions and sizes to resolve those anchors. The drawing_unmodeled_extents diagnostic marks rotations/effects or legacy objects whose additional extent is not modeled; no COM fallback is requested. Legacy helper functions remain for older code paths, but Expand-RustVisualCaptureRange no longer calls them.

## Rust bounds plus right-side scan (current behavior)
The initial capture range uses only Rust max_row/max_column (values, merges,
and drawing cell markers). PowerShell no longer resolves drawing_points through
cell Left/Top or Find-VisualEndIndex. Both legacy COM shape entry points now
throw instead of enumerating Shapes. Remaining COM range dimensions are for
image capture/tile sizing, not drawing geometry.
Two-cell anchors contribute both markers. One-cell anchors currently contribute
the origin cell; their physical extents and absolute-anchor physical coordinates
remain diagnostic data and are not converted to end cells. This is an initial
approximation, not guaranteed full coverage of those anchor types. The right-side
pixel scan covers only max_row and stops at the first blank visible column.
Screenshot cache version is 12 so earlier capture bounds cannot hide this change.
PowerShell tests assert no drawing geometry resolution, including when an old
plan still contains drawing_points and drawing_com_fallback=true. Real workbook
speed/GENESIS coverage checks and Rust compilation are left for the WSL run.
## Excel-saved dimensions (current pre-capture stage)
Before screenshots, assert shrink package invariants on the raw shrink artifact.
Create a separate `excel-automation/shrunk-excel-saved.xlsx` by opening a local
copy in Excel, saving in manual calculation mode with CalculateBeforeSave=false,
and closing it. Do not overwrite raw shrink output or its cache object.
`dimension-save-progress.log` records open/save/close timings, and
`dimension-save-package-changes` records Excel's package rewrites.
Compare raw versus saved cell values strictly before using the saved file for
screenshots. Formula serialization is allowed to change in this save stage;
raw shrink formula preservation remains covered by the preceding validation.
The regular Rust Cells/Summary pass records dimension and the value/formula
rectangle (including hidden cells). Only the Excel-saved side applies dimensions:
missing/invalid dimension or one not containing values is an error. Overhang
larger than the value rectangle's height or width warns, but retains that bound.
Empty sheets use the dimension without an overhang warning. Final capture bounds
combine prior value/merge bounds, saved dimension, and both workbooks' Rust
Drawing cell-marker bounds. No COM Shapes or drawing geometry fallback is used.
Screenshot cache version is 13. Value-summary code hashes invalidate old summaries.

Validation: PowerShell syntax checked; Rust tests added in value_views.rs and
value_views_tests.rs. Run in WSL:

```bash
cargo test -p excel-shrink-cli dimension
cargo test -p excel-shrink-cli regression::xlsx_value_compare
```

A Windows COM smoke attempt on 2026-09-27 failed during preparation with Excel's
insufficient memory/disk-space error before producing a saved workbook; the actual
save/reopen behavior still needs an integration run in an available Excel session.