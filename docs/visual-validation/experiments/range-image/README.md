# Get Picture experiment

> Current runs: see [consolidated capture/report contract](../CONSOLIDATED-CAPTURE.md). Output is now central
> at `docs/visual-validation/results/`. Historical paths and monolithic timing notes below
> describe the earlier experiments, not the current safe range-image capture path.


This experiment compares ways to turn visible Excel worksheets into PNG screenshots.
It is deliberately self-contained below `docs/visual-validation/experiments/` and
does not depend on production `tools/` code.

## Office.js strategies

- **A — UsedRange prefetch:** obtain every visible sheet's `UsedRange` in one planning
  pass, measure that pass, then capture images.
- **B — UsedRange inline:** resolve `UsedRange` immediately before each image batch.
- **C — content + drawings + margin:** use value-only content bounds, visible drawing
  envelopes, map point bounds back to worksheet cells, and add 10 rows/columns.
- **D — horizontal overflow XOR:** use C's content/drawing bounds without the fixed
  right margin, photograph a screen-width probe to the right, fill the probe with
  whitespace, photograph again, restore contents, and let the host XOR the pair in
  a worker. The final right bound is calculated from the changed pixels.
- **E — horizontal + vertical overflow XOR:** D plus an analogous screen-height probe
  below the content.
- **F — WSL Rust OOXML bounds + horizontal render overflow:** before either Office.js
  host starts, a small native Rust preplanner runs inside WSL and scans worksheet XML for
  content/merge bounds, column definitions, trailing-hidden column/row tails, and drawing
  anchors. It writes a host-neutral JSON plan that is staged with the add-in. Office.js
  consumes that plan instead of asking Excel for UsedRange/hidden tails/normal drawing
  bounds. Only sheets that still need it get a horizontal render probe. The task-pane
  WebView analyses and crops that same PNG with Canvas, so there is no whitespace blocker,
  XOR pair, vertical overflow probe, second final capture, or strategy-specific host code.
  Unsupported drawing anchors use a narrow Office.js shape fallback on those sheets only.
  Strategy F therefore works with both the Rust and ASP.NET Office.js hosts.

For strategies C-E, trailing hidden worksheet geometry takes precedence over content,
drawing, margin, and overflow planning on the affected axis. The planner first checks
the terminal column/row (XFD / row 1,048,576). If it is hidden, it binary-searches the
first column/row of the all-hidden suffix. Hidden columns/rows in the middle of visible
geometry do not create a bound; a visible terminal column/row means there is no
trailing-hidden bound. When an axis is fixed this way, content/drawing discovery and
overflow probing for that axis are skipped.

D/E persist each overflow probe's before, after, and XOR PNG below the workbook's
`_overflow-probes/` image directory, and the result JSON records those relative paths
together with the pixel/extent decision.

F does not persist intermediate overflow probes. The current probe PNG becomes the final
artifact when the task-pane Canvas analysis can close the right edge; it is cropped in
the WebView and only widened/re-rendered when content reaches the current probe edge.

Office.js intentionally captures each final sheet as one `Range.getImage()` call,
including extreme aspect ratios. The VBA baseline may tile internally.

## Hosts

The same web add-in runs against either:

- `Office.js · Rust host`: Windows Rust executable cross-compiled from WSL/MinGW.
  The default launch mode opens/closes workbooks through the existing `excel-com`
  experiment dependency.
- `Office.js · ASP.NET host`: .NET 8/Kestrel reference implementation.

Both hosts expose the same protocol and result schema.

## Timings and activity

Planning, rendering, transfer/persistence, and probe analysis are recorded separately.
The task pane sends startup events and a 5-second heartbeat. The runner has a long
startup timeout, an inactivity timeout that resets on meaningful host/add-in activity,
and a separate absolute safety timeout.

A successful `/result` closes the workbook immediately; normal benchmark runs no
longer sleep for five seconds after completion.

## UI

The task pane contains deliberate top padding so Excel's one-time add-in notification
does not cover the experiment state. It shows host connectivity, workbook/phase,
progress, current batch sheets, batch timings, average milliseconds per image, ETA,
overflow-analysis progress, and a collapsible technical event log.

## Running

Use the single VS Code task:

```text
Experiment: visual validation
```

or directly:

```powershell
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass `
  -File ./docs/visual-validation/experiments/run.ps1
```

The terminal dispatcher asks for experiment, strategy, implementation, and—when
relevant—Office.js batch size.

The runner is also scriptable, for example:

```powershell
./docs/visual-validation/experiments/range-image/run.ps1 `
  -Strategy D `
  -Implementation officejs-rust `
  -BatchSize 8 `
  -OverflowProbeWidthPx 1920 `
  -NonInteractive
```

## VBA scope

The existing VBA `CopyPicture -> Chart.Paste -> Chart.Export` path remains the
reference baseline. In this refactor it is currently wired to strategy A's all-visible
sheet plan. Strategies B–F are the Office.js planning experiments; their VBA
equivalents can be added later without changing the shared Office.js protocol.
