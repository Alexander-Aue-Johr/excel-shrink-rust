# Targeted comparison

The comparison cards and their acceptance contract are documented in
[Targeted comparison cards](../../targeted-comparison-cards.md). The Observatory
can now explicitly start this bounded workflow on private copies. Scientific
coverage limits below still apply.

`excel-shrink-cli regression targeted-compare` compares one original XLSX with one
Default shrink XLSX, plus Excel Check Performance on a separate Original copy,
without starting the corpus runner or Observatory. It copies both inputs, keeps
each enabled copy open in its own Excel instance, and writes PNGs,
`report.json`, and `telemetry.json` to a new output directory. No blanket sheet
capture is made.

```powershell
excel-shrink-cli.exe regression targeted-compare `
  --original C:\path\original.xlsx `
  --default C:\path\default.xlsx `
  --output C:\path\new-output-directory `
  --skip-check-performance `
  --host-script C:\path\excel-shrink-cli\office-capture\targeted-comparison-host.ps1
```

The standalone example explicitly omits Check Performance. By default it is
**enabled** and requires the private-desktop launcher (as used by the Observatory).
The native adapter refuses the interactive Default desktop; it sends no global
mouse or keyboard input. It requires a supported English Excel performance pane.
Missing UI or unconfirmed completion fails with a bounded command deadline.

`--observatory-group-dir` can point to an existing `groups/<id>` directory. The
command then writes `targeted-comparison.json` there for the Observatory group
view. This is an explicit option; the standalone command never starts a run.

## Implemented path

1. Compare cell XML in matching worksheets. Empty styled-cell changes collapse
   to one row geometry probe instead of thousands of screenshots.
2. Open Original (`O`) and Shrink Default (`SD`), save both, and add cell changes
   produced by the ordinary Excel save.
3. Make a third copy (`CP`) from the normally saved Original. Open/save this copy
   and record its additional ordinary-save changes separately. Invoke Check
   Performance / Optimize all, require positive completion text, save, and compare
   XML against the CP pre-optimization snapshot. Extend the shared candidate queue.
4. Capture each candidate with one neighboring cell on every side. Read its
   rendered position and size. Rebuild and capture SD and CP, then rebuild and capture O.
5. Compare PNG pixels and preserve all images and geometry in the report. The
   `also_changed_by_original_recalc` field identifies recalc drift in O.

The current code is an experiment, **not complete visual validation**. It
stops after 64 changed-cell candidates. It does not yet discover chart, drawing,
style, row, column, or all formula-dependent changes. It does not detect new
candidate cells introduced only by recalculation. Empty-cell changes are
mapped to column A in the same row; this catches row-height effects but does
not prove every visible consequence. Reopening after save is not checked.
The report records these limits; it must not be read as a clean comparison of the
whole workbook. A completed native optimization does not certify visual equality.
`check_performance_evidence` retains native completion text, immutable before/after
optimization copies, separately attributed save/optimization candidates, and CP
findings with image paths and geometry for all four control states. Default's
existing findings stay separate. `partial-report.json` retains completed evidence
when a later phase fails. A null CP findings/difference count means comparison
was not performed; it does not mean zero differences.

## Local integration check

On 2026-10-31 the standalone Windows executable compared the synthetic
`baseline.xlsx` and `removed-empty-cell.xlsx` from the row-top experiment.
It selected one candidate, `Rows!A500000`, and captured
`A499999:B500001` in four states. The original and changed copy differed
before recalculation and after rebuilding the changed copy; rebuilding the
original did not change its captured region. The original row height was
53.25 points in both captures. This verifies the basic control sequence and
the collapse of 16,384 empty-cell removals to one row probe. It does not test
Check Performance, drawings, or production corpus coverage.

## Native Check Performance lane

The tested native trigger is now shared with the
[trigger experiment](../check-performance-trigger/README.md). The orchestration
lives in `targeted_comparison/check_performance.rs`; the platform adapter is
`office-capture/CheckPerformance.psm1`. The top-level workflow explicitly invokes
optimization, CP captures, CP rebuild and CP evidence comparison. It does not use
Office.js to invoke Check Performance: no suitable public API was established.

Even identical Original/Default XML does not skip CP. Its optimization can expose
new candidates. All enabled lanes capture the resulting union. A missing CP
image, unavailable command or unconfirmed optimization is an error, never an
identical result. `--skip-check-performance` explicitly omits the lane; existing
records marked unavailable remain historical records and are not upgraded.

Before using this for a corpus, replace the in-memory cell map with a bounded
streaming comparison, add complete candidate discovery and report filters, and
connect actual Office.js heartbeat/progress to the Observatory cards. The
cards now distinguish actual phase/capture progress from externally measured
PID-liveness samples. This experimental host still uses COM screenshots and
does not emit Office.js add-in heartbeats. Full candidate discovery and the
streaming cell comparison remain future scientific work.

## Card and lifecycle verification

`run-smoke.ps1 -Output <new-directory>` creates a tiny font-height counterexample
and runs the actual comparison on an owned private desktop. Add `-FailCapture`
to inject a capture error after real Excel startup and verify cleanup.
`-CheckPerformance` uses identical inputs with redundant styled cells, requires a
real Optimize all completion, checks `Z10000` was removed in saved XML, compares
CP before/after rebuild against Original, and verifies all three process exits.
`-FailOptimization` injects an unconfirmed optimization response after real Excel
startup; the command must fail explicitly and close every owned Excel process.
`-CleanWorkbook` verifies a positive "No optimization needed" outcome without
attributing ordinary save changes to optimization.

`test-panel.cjs <new-output-directory> <telemetry.json>` runs an isolated headless
Edge with synthetic UI verification frames derived from that telemetry. It
checks the real shaders, retained canvases, workflow matrix, history and
fullscreen. It requires Node with built-in WebSocket support; the installed
VS Code runtime can be used with `ELECTRON_RUN_AS_NODE=1`.

`test-host.cjs <smoke-directory>` verifies the actual HTTP launch path using a
`prepared/smoke.zip` fixture with Original/Default inputs. Its fixed test port is
8879. It does not use or restart a production host.
