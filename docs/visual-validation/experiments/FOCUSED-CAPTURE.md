# Focused capture reproduction

Status: active engineering workflow; **not a production strategy approval**.

This entrypoint uses the existing Strategy F engine, workbook-copy isolation, process
supervisor, pixel checks, reports and verified metadata bundle. It does not introduce
another renderer or silently narrow a full-corpus qualification run.

## Start with one known issue

From Windows PowerShell, in the repository root:

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -CaseId kernhaushalt-long `
    -StartupTimeoutSeconds 300 -CaptureTimeoutSeconds 1800
```

Only `csv-71717-22` is selected. The user reports 12,355 real rows; that information
is recorded as context, **not hard-coded as the capture bound**. Excel still opens
the workbook and Rust still prepares its workbook metadata. The other worksheets
are not calibrated, captured or whitespace-probed. No worksheet is deleted from
or resized in the working copy. Gridlines are changed only on selected sheets.

The existing VS Code task **Visual validation: Check capture fixes (focused)** now
opens the focused WPF selector. The main visual-validation GUI also has a
**Focused checks…** button. CLI execution does not load WPF.

## Named cases

| Case ID | Exact scope | Remaining question |
| --- | --- | --- |
| `kernhaushalt-long` | Kernhaushalt: `csv-71717-22` | Complete the long sheet without spending the deadline on preceding sheets. |
| `wanderungen-drawings` | Wanderungen: `Vorbemerkung`, `Anhang` | Record shape point coordinates and final native grid brackets; reject an unproven last-row fallback. |
| `wanderungen-single-column` | Wanderungen: `Qualitätsbericht` | Record bounded single-width controls; do not pretend that a full X calibration was fitted. |
| `verkehrsunfaelle-start` | Verkehrsunfälle: `1.1` | Separate startup from processing; then inspect the additive-height capture. |
| `seeschifffahrt-start` | Actual first sheet of Seeschifffahrt | Resolve its real name and inspect startup on a minimal selected scope. |

```powershell
# These commands do not launch Excel.
& .\docs\visual-validation\experiments\decision-campaign.ps1 -Focused -ListCases
& .\docs\visual-validation\experiments\decision-campaign.ps1 -Focused -CaseId kernhaushalt-long -ListSheets
& .\docs\visual-validation\experiments\decision-campaign.ps1 -Focused -CaseId kernhaushalt-long -PlanOnly
```

`-ListSheets` reads only the bounded `xl/workbook.xml` catalogue, not cell XML or
UsedRange. The listing includes original 1-based workbook positions and visibility.
Exact names preserve whitespace. A hidden worksheet selection fails before Excel,
instead of unhiding it or silently running another sheet. Missing or ambiguous
workbook selectors also fail before execution.

## Custom scope

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Workbook 'wanderungen-2010120217005.xlsx' `
    -Sheet @('Vorbemerkung', 'Anhang') `
    -HostKind dotnet -OverflowMode selective -TileMegapixels 16 `
    -StartupTimeoutSeconds 300 -CaptureTimeoutSeconds 1200
```

`-Workbook` accepts a corpus-relative path, an absolute path inside the corpus,
or an unambiguous exact filename. There is deliberately no implicit wildcard
expansion. `-SheetIndex @(1, 3)` is an alternative to `-Sheet`; indexes are actual
workbook order, including hidden sheets in that numbering. The add-in retains
workbook order for capture. An omitted sheet selector explicitly means all visible
sheets in the selected workbook, not all corpus files.

Use `-CaseId @('wanderungen-drawings', 'wanderungen-single-column')` to run two
independently reported cases on the same workbook. Neither receives the other
case's previous results. Each case uses a fresh working copy.

For native shell calls through `powershell.exe -File`, or several custom workbook
selections, use **a request file rather than interpolating array arguments**:

```json
{
  "schema": "vv-focus-request-v1",
  "targets": [
    {
      "id": "drawings",
      "workbook": "original_destatis_xlsx_files/wanderungen-2010120217005.xlsx",
      "sheets": ["Vorbemerkung", "Anhang"]
    },
    {
      "id": "fixed-width",
      "workbook": "original_destatis_xlsx_files/wanderungen-2010120217005.xlsx",
      "sheets": ["Qualitätsbericht"]
    }
  ],
  "host_kind": "dotnet",
  "overflow_mode": "selective",
  "tile_megapixels": 16,
  "startup_timeout_seconds": 300,
  "capture_timeout_seconds": 1200,
  "diagnostic_level": "detailed"
}
```

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -SelectionFile '.\focus-request.json'
```

CLI overrides take precedence over request-file values. `-Workbook` or `-CaseId`
replaces the selection; names/indexes are never merged accidentally. The exact
resolved request and effective timeouts are archived before the case begins.
WSL receives paths to UTF-8 JSON files, not executable shell fragments.

## Timeouts and configuration

All numbers below are seconds. The same settings are available in the focused GUI.

| Parameter | Default | Meaning |
| --- | ---: | --- |
| `SetupTimeoutSeconds` | 600 | Worker preparation, plans, build and host start, before workbook launch. |
| `StartupTimeoutSeconds` | 300 | Excel launch to the add-in `measurement_started` event. |
| `CaptureTimeoutSeconds` (alias `TimeoutSeconds`) | 1200; 1800 for `kernhaushalt-long` | Measurement start to final capture result. Supports up to 7200. |
| `NativeCallTimeoutSeconds` | 90 | One instrumented Office call. A native timeout poisons the session; a longer total allowance does not waive it. |
| `InactivityTimeoutSeconds` | 120 | Missing add-in activity after capture starts; at least native-call timeout + 10. |
| `PostprocessTimeoutSeconds` | 240 | Offline verification/report processing, also bounded when capture ends with an error. |
| `SessionTimeoutSeconds` | Derived, finite; at least one hour | Absolute overall cap; never extended by heartbeats. |
| `Repetitions` | 1 | Independent repetitions, 1–5. No hidden warmup or host/MP sweep. |

Setup + startup + capture + 30 seconds is the absolute workload ceiling. It is
**not** a shared six-minute countdown; phase budgets are checked separately.
The session budget must fit at least one configured case plus postprocessing and
cleanup. A shorter explicit value is rejected before Excel, not silently applied.
Final process cleanup can take an additional bounded interval after cancellation.

Other settings: `HostKind` (`dotnet`/`rust`), `OverflowMode`
(`selective`/`heuristic`), `TileMegapixels` (16/32/64), `ContextValidation`
(`off`/`first-seam`), `DiagnosticLevel` (`normal`/`detailed`), `WaitForStart`,
`UseOfficeDebugJs`, and `CorpusRoot`. `ContextValidation=off` omits only the extra
local reference experiment; visible context halos, strict geometry checks and
canonical seams remain enabled. Batch=1 and budget-aware packing remain fixed.

Selecting 32 or 64 MP runs exactly that configuration; this focused workflow does
not hide failures behind automatic 16-MP rescues or choose a winner. A comparison
under altered deadlines remains diagnosis, not a replacement for the qualification
measurements.

## Evidence and failure behavior

Every managed run retains the existing reports and publishes one checked
`review-metadata.zip`. Final and raw PNGs remain local. The focused report adds:

- exact selections, resolved original indexes, phase budgets and capture configuration;
- `capture-lifecycle.json` and an append-only `.jsonl` timeline with startup milestones,
  last sheet/range/native call, received-image counts and owned Excel process metadata;
- process ID, start identity, CPU time, memory, responding flag and main window title;
- per-case `focus-request.json` and `reproduce.ps1` for a new identical reproduction;
- detailed drawing-to-grid source coordinates, bounded probes and final bracket checks;
- up to four small calibration-diagnostic PNGs when a degenerate axis cannot be fitted.

The observer does not attach through COM, capture the desktop, dismiss dialogs,
change security settings or terminate unrelated processes. Main window metadata
can help distinguish waiting from processing, but is not guaranteed to identify
all modal-dialog or add-in initialization failures. Titles and logs can contain
local paths and workbook names; metadata bundles are not anonymized.

A drawing target without a proven grid bracket becomes `DRAWING_BOUND_UNRESOLVED`
on that sheet. It is no longer treated as permission to capture the last Excel
row. The smaller Rust bounds are **not silently accepted instead**. The old native
point-to-grid search is instrumented and checked here, not replaced by a newly
claimed-correct shape geometry model.

Single-width observations are diagnostic only. This patch does not falsely report
`Qualitätsbericht` as calibrated; the observations are intended to support a bounded
fixed-width implementation next. Neither increased budgets nor additional controls
can convert a pixel mismatch into success.

Normal worksheet errors remain failures and use the existing health-checked
continuation policy. A poisoned native session is stopped. Shared deployment
failures block the remaining cases. After cleanup succeeds, other independent
focused cases can proceed. If ownership is uncertain, execution pauses.

## Rerun, resume, and repair reports

A focused command starts a **new report by default**. This avoids resuming an old
completed failure and doing no work. The GUI remembers its last request under
`.tmp/visual-validation/focus-last-request.json`.

`reproduce.ps1` in a case directory reruns just that case into a new managed report.
The root replay script reproduces the selected set. A request file also works from
another shell without copying long path arguments.

`-ResumeRun <existing managed directory>` without selection overrides reloads that
run's resolved configuration and continues pending/interrupted work only. Source
and input fingerprints must still match. Completed failures are not silently retried
or erased; use a new focused run for them. No new configuration is mixed into old
measurements.

`-Focused -ReportOnly -ResumeRun <directory>` rebuilds the focused report and bundle
without starting Excel. It does not create absent screenshots or certify missing
pixel checks.

## Which experiments to run now

| Entry | Status / use |
| --- | --- |
| Focused capture checks | **Active:** isolate the unresolved sheet or startup case. |
| Finalize capture strategy | **Deferred:** run after the common capture issues are resolved. Full readiness is still required. |
| Run comparison suite | Retained historical/repetition tool. Do not rerun it to diagnose one sheet. |
| Decide strategy (all workbooks) | Retained broad inventory. Not the next troubleshooting step. |
| Old `capture-followup.json` six-file preset | Historical repair selection, retained for old requests/tests; the interactive focused entry now uses `capture-focus-cases.json`. |
| Units, row-height and plateau fixtures | Retained diagnostics. Rerun when their renderer assumption changes, not for a launcher or selection patch. |
| Original outside-edge seam rule | Superseded by visible-cell context plus interior crop; old incident evidence remains. |
| Native VBA/GDI+/C# drawing references | Retained, with their documented different scope and unresolved integration failures. Not declared equivalent screenshot winners or deleted as useless. |

No experiments, old reports, baselines or source corpora are deleted by this patch.
A successful selected-sheet run is not automatically imported as full-workbook
readiness and does not release the 10,000-file original-versus-shrunk regression run.

## Validation boundary

Pure tests cover exact metadata selections, whitespace/index identities, invalid
requests, phase-budget propagation, replay, focused controller/report behavior,
shape bracket rejection and diagnostic-only single-axis observations. Installer
checks cover source conflicts, backups, formatting, idempotence and rollback.
They are not native Windows/Excel tests.

Before an actual focused case, the Windows entrypoint parses the involved PowerShell
files, checks parameter contracts and evaluates phase-deadline examples. It then
runs the existing six native pixel checks. A failure stops before Excel. WPF must
still be opened on Windows, and real Office startup/capture behavior remains to be
measured on the selected files.


## Fixed-width repair and question profiles

Use `-Focused -Profile fixed-width` for the three remaining Wanderungen sheets,
with two independent repetitions and no subsequent matrix. The GUI exposes the
same **Question profile** selector. Budget, host, overflow, startup and remaining
coverage scopes are prepared as separate finite profiles, not run automatically.
See [fixed-width scope, commands and evidence rules](FIXED-WIDTH-AND-QUESTIONS.md).
