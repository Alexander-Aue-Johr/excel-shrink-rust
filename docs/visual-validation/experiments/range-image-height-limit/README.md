# Range.getImage height plateau (diagnostics v2)

> Current runs: see [consolidated capture/report contract](../CONSOLIDATED-CAPTURE.md). Output is now central
> at `docs/visual-validation/results/`. Historical paths and monolithic timing notes below
> describe the earlier experiments, not the current safe range-image capture path.


This experiment replaces the earlier exception/no-exception binary search. A valid PNG
header does not prove that the requested range was rendered at full size. In the supplied
run, `A1:FX2791` returned a PNG of 15440 x 32766 pixels, although its Office.js height was
35596.5 points (47462 pixels at an assumed 96 DPI).

## Scope

Only `krankenhausverzeichnis-3500100227005.xlsx`, worksheet ` KHV_2022` (leading space),
is opened. The preferred source is `corpus-local/original_destatis_xlsx_files/`.
The source workbook is never rewritten. The existing launcher opens a disposable copy.
The main search fixes columns A:FX and varies the bottom row between 1 and 2791.
There is no production tiling, resizing, overflow analysis or worksheet mutation here.

## Method

1. Capture rows 1, 40 and 100 separately. Fit `PNG pixels = slope * Office.js points + intercept`.
   The intercept is measured, not forced to zero or one. Reject an unreliable baseline fit.
2. Capture the full range twice. Learn its candidate plateau height from the returned
   PNG, rather than hard-coding 32766. Compare to the small-range fit, not only 96/72.
3. Binary-search the first row whose returned PNG height equals that candidate. A smaller
   PNG must agree with the baseline model to count as below the plateau. Rendering errors,
   unexpected dimensions and other scale anomalies are **unknown**, not a plateau result.
4. Probe every row within +/-5 rows of the candidate transition and repeat the two adjacent
   boundary rows in alternating order. Require the same plateau for multiple distinct
   requested physical extents. Report contradictions as `inconclusive`.
5. After the A:FX search, repeat three selected heights with A:H and A:AZ, labelled explicitly
   as separate width controls. They do not alter the primary search predicate.

The result is row-resolution, locally checked evidence for this file and Excel build,
not a proof of global monotonicity, a pixel-exact universal limit, or a PNG format limit.
The experiment does not conclude clipping versus compression from dimensions alone.
Use the separate synthetic marker experiment for that question.

`Range.getImage` success here means signature/IHDR/positive dimensions/IEND checks passed;
it is not a full CRC or image-content validation. Hospital PNGs are not uploaded, persisted,
or decoded to browser canvases. The returned Base64 is released after inspecting the header.

## Run

Use **Experiment: visual validation -> GUI -> Get picture height limit / plateau**, or:

```powershell
.\docs\visual-validation\experiments\range-image-height-limit\run.ps1
```

Optional switches: `-WaitForStart`, `-BreakOnError`, `-UseOfficeDebugJs`.
Extended error logging is always enabled. Breakpoints use real `debugger;` statements,
conditioned on `-BreakOnError` or `window.VV_BREAK_ON_ERROR = true` in DevTools.
For deliberate debugger pauses, also enable Wait for Start. Pause time is not render time.

The new VS Code task **Experiment: PNG diagnostics (choose test)** is another entry point;
it runs the selected diagnostic directly with default options.

## Output and reliability

Latest result: `results/officejs-dotnet-height-limit.json` plus a sibling CSV.
Every run also has an independent archive under:

```
.tmp/visual-validation/range-image-height-limit/runs/<UTC-time-and-id>/
    result.json
    probes.csv
    files.txt
    diagnostic-config.json
    previous-latest-result.json             # when a previous result existed
    artifacts/_diagnostics/
        <workbook>.progress.json
        host.stdout.txt
        host.stderr.txt
```

Progress is saved after every completed attempt, including failures. An in-progress native
Excel call cannot be forcibly cancelled by a JavaScript Promise timeout: the diagnostic
stops queuing further Office calls and reports inconclusive. Default per-probe timeout is
240 seconds; this is a diagnostic safety budget, **not evidence of a rendering limit**.
A small recovery control and one isolated retry are used after a normal render error.
If that control also fails, the remaining run stops.

Run-ID checks reject writes from stale task panes. Only one pane can claim a workbook.
The launcher closes only the Excel process it started and retains diagnostic artifacts on
failure. Inspect `status` and each `analysis.*.conclusion`, not only terminal exit code.
A completed diagnostic can still yield `no-saturation-observed-up-to-test-limit` rather
than a discovered cap. If setup fails, there may be no final result; use the run archive.
