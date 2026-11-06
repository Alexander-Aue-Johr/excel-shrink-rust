# SheetScope operating guide

SheetScope runs the existing targeted spreadsheet comparison independently of
Excel Shrink's cleanup command. It can compare existing A/B workbooks, run a
configured sequence of external processors, or open a previously saved report.
Excel Shrink remains available as `excel-shrink-cli`; no repository split is
required.

A completed comparison describes the observations made under the recorded
capture policy: selected worksheet regions, layout checks and charts. It does
not claim that every pixel of the entire workbook was rendered. The independent
report retains that scope, its limitations and any reused observations.

## Prepare the capture machine

For new visual comparisons, use a Windows executable, desktop Microsoft Excel
and Windows PowerShell. The executable can be built natively on Windows or
cross-compiled from WSL. Keep the repository's `excel-shrink-cli/office-capture`
assets available, including the private-desktop launcher, its C# helpers and
Office.js assets. Running from the repository root is the simplest way to retain
the established asset lookup. These are the existing capture components;
SheetScope does not replace Excel with a different renderer. HTTP(S) archive
sources also require the existing system `curl` executable.

In VS Code, choose **Terminal → Run Task → SheetScope: Start Observatory**.
The task first builds the Windows release, then starts it without workbook
arguments and opens the configuration page in the browser. Choose sources,
processing steps and a new report location there. **SheetScope: Build Windows
release** is also available as a separate task. In a WSL workspace, it uses the
`x86_64-pc-windows-gnu` target; the start task copies the executable and capture
assets into a versioned local Windows directory before launching them. In a
Windows workspace, the task builds with the native Windows toolchain.

For the small local Destatis corpus, choose **SheetScope: Test Destatis
(prefilled)**. It builds both executables, selects
`corpus-local/original_destatis_xlsx_files`, fills in the deployed Excel Shrink
executable and opens configuration. Click **Start validation** to begin; the
launcher itself does not process any workbooks. In a WSL terminal the equivalent
is:

```bash
bash tools/start-sheet-scope-wsl.sh -Destatis
```

The starter retains each report in a unique
`%LOCALAPPDATA%\SheetScope\runs\destatis-<id>\report` directory. Processed
workbooks are temporary and removed after evidence publication. It does not
create the report destination in advance. Local Windows working storage avoids
Windows file-lock limitations on WSL network paths.

You can also prefill configuration directly with CLI arguments:

```powershell
.\target\x86_64-pc-windows-gnu\release\sheet-scope.exe observatory --a "D:\Originals" --excel-shrink "D:\Tools\excel-shrink-cli.exe"
```

Omit `--output` for temporary workbooks and `--report` for a generated unique
local report path. Optional `--max-processes` and `--rayon-threads` set the external
process count and threads per process separately. The preset defaults to as many
external processes as logical processors, with one Rayon thread per process.
The configuration shows both settings, along with typed Excel Shrink options
generated from its bundled Clap definition; other programs use the argv editor.

Build both command-line entry points from the repository root:

```powershell
cargo build --release -p excel-shrink-cli --bins
```

The resulting Windows executables are:

- `target/release/excel-shrink-cli.exe`: compatible cleanup and legacy regression
  commands.
- `target/release/sheet-scope.exe`: independent validation, configuration and
  report commands.

Building requires the existing Rust/Windows build toolchain. This guide gives
operating commands, not a claim that the patch was built or exercised with Excel
in the SCE editing environment. Viewing an already generated static report
requires only a browser; it does not require Excel, PowerShell or either CLI.

## Start with the Observatory

No workbook paths or processor arguments are required to open configuration:

```powershell
.\target\release\sheet-scope.exe
```

The command starts a loopback server and opens its local address in the browser.
The explicit equivalent and a configuration-preloaded variant are:

```powershell
.\target\release\sheet-scope.exe observatory
.\target\release\sheet-scope.exe observatory --config .\sheet-scope.json
```

Use `--port 8765` to request a particular port or `--no-open` to print the address
without launching a browser. The default port `0` asks the operating system to
select a free port.

Choose **Compare existing A and B**, **Process A, then compare**, or **Open a saved
report**. Native Windows folder/file dialogs select local sources, programs and report
locations; an archive URL can be entered directly. Choose a new report directory
for each run. For processing, add steps in execution order and configure each
step's input/output granularity and argv array. The Excel Shrink preset supplies
its actual directory-output contract; select the executable explicitly.

Record an **environment key on the first baseline run** if later visual-result
reuse is planned. Use a meaningful identifier for the Excel build, fonts and
other rendering assumptions under which that observation was collected. The key
is your declaration, not an automatically measured environment fingerprint.
Change it when those assumptions change.

**Start validation** begins the run and closes configuration. The monitor remains
visible behind it before startup, with empty counts and lists. It reuses the
Observatory's KPI styles and targeted process cards, heartbeats and workflow
diagram. The workbook list supports search, result filters and pages of 100.
The view shows the current phase/workbook, capture telemetry and the latest 20
workbook results. A finished
workbook's comparison page becomes available while later workbooks continue.
The complete report indexes every recorded workbook when the run ends.
Configuration is fixed for the running attempt; this patch does not add runtime
capture-tile or hash-mode controls.

This is not the complete legacy corpus-runner interface: its live controls and
three-dimensional studio are not connected to standalone runs. Published pair
evidence opens inline with the shared region viewer and pixel inspector.

Use **Download configuration** to retain the form as JSON. **Edit complete JSON**
also exposes advanced settings such as per-step environment overrides, working
directories and declared dependency files. Closing the browser tab does not
cancel the backend run; keep the SheetScope process running until it finishes.

## Compare existing files directly

A and B can independently be local directories, individual workbook files,
local ZIPs or HTTP(S) ZIP URLs. They need not use the same source type. This
example compares two existing directories without invoking a processing program:

```powershell
.\target\release\sheet-scope.exe compare --a "D:\corpora\originals" --b "D:\results\shrink-v1" --report "D:\validation-reports\baseline-v1" --environment-key "office-lab-2026-11-a"
```

`--environment-key` is optional for a first run, but setting it then is necessary
if that report will later be reused under a nonempty key. Omitting it does not
prevent an ordinary fresh comparison.

File matching is deterministic and case-insensitive. Relative names take
precedence, followed by unique basenames; the matcher then considers extension
changes. This accommodates different archive wrapper directories without
choosing arbitrarily between repeated names. Ambiguous or missing partners are
reported, and extra B files are listed separately. The current targeted renderer
requires XLSX on both sides; acquisition of other Excel formats does not remove
that requirement.

Add `--continue-on-error` to continue with other file pairs when a partner is
missing or a comparison fails. The option controls continuation, not the final
exit code: a run containing failed comparisons still returns an error after
publishing its available report. Unconfirmed process cleanup always stops new
work.

The short `compare` command exposes common direct-comparison options. Use a
JSON configuration through `run --config` or the Observatory for the full set of
download/cache and capture settings.

## Process with Excel Shrink, then compare

Save this example as `sheet-scope.json` in the repository root, as UTF-8 JSON
without a byte-order mark. Adjust the corpus and report paths to the machine.
The `program` path below is relative to that configuration file.

```json
{
  "source_a": "D:/corpora/originals.zip",
  "steps": [
    {
      "name": "Excel Shrink",
      "program": "target/release/excel-shrink-cli.exe",
      "input": "file",
      "output": "directory",
      "arguments": [
        "{input}",
        "{output_dir}",
        "--max-active-workbooks", "1",
        "--rayon-threads", "1",
        "--writer-workers", "1"
      ],
      "timeout_seconds": 3600
    }
  ],
  "report_dir": "D:/validation-reports/shrink-v1",
  "output": { "kind": "temporary" },
  "cache_dir": "D:/validation-cache/downloads",
  "max_processes": 2,
  "continue_on_error": false,
  "environment_key": "office-lab-2026-11-a"
}
```

Execute it without opening the configuration UI:

```powershell
.\target\release\sheet-scope.exe run --config .\sheet-scope.json
```

Excel Shrink takes an XLSX file or a directory as input and an **output
directory** as its second positional argument. It does not accept a corpus ZIP
in its ordinary cleanup command. SheetScope therefore extracts the selected
source, starts one Shrink invocation per workbook in this example, and assigns
the appropriate final output parent. The explicit Shrink worker arguments avoid
multiplying its internal concurrency by the outer process count.

To invoke Shrink once for a whole directory instead, change `input` to
`"directory"` and use arguments `["{input}", "{output}", "--recursive"]` plus the
wanted cleanup options. Outer `max_processes` applies to file invocations;
directory/archive invocation runs one process for the stage. A batch stage may
need a longer timeout than the default one hour.

Other processors can use file, directory or ZIP input and file, directory or
ZIP output, subject to the stage contract. Arguments are individual strings,
not a shell command. Additional switches are simply additional array entries.
See [source and processing contracts](validation-processing.md) for placeholders,
output verification, subprocess ownership and failure logs.

The complete default configuration can be printed for editing:

```powershell
.\target\release\sheet-scope.exe template
```

### Relative paths

Structured path fields loaded by `--config` resolve relative to the JSON file's
directory. This includes source paths, report/output/cache paths, processor
executables, working directories and declared dependency paths. Paths typed or
pasted into the Observatory resolve relative to the SheetScope process's
working directory; the interface displays that directory. Browsing normally
provides absolute paths. If a downloaded configuration still contains relative
paths, placing it elsewhere changes their meaning when later loaded by CLI.

Text within `arguments` or `environment` remains literal except for supported
placeholders. It is not automatically rewritten relative to the configuration
file. Set `working_directory` explicitly, or use an absolute script/resource
argument, when a processor expects relative paths there.

### Conversion chains and the visual baseline

Steps execute in their array order, and `output_extension` can change the
expected workbook suffix while preserving its relative name. For an XLS-to-XLSX
converter followed by Excel Shrink, set:

```json
"baseline_after_step": 1
```

This selects the first stage's output as A and compares it with the final stage's
output. The number is **one-based** and must select an intermediate stage: it
must be greater than zero and less than the total number of steps. Without this
setting, A remains the initially supplied source.

The example consequently validates transformations **after conversion**. It
does not visually validate the XLS-to-XLSX conversion itself. Supplying XLS as
the original baseline to the XLSX-only targeted engine is rejected rather than
silently treated as covered.

## Keep inputs, processor outputs and reports separate

Report and permanent output destinations must be new paths. Keep report,
download-cache, processing-cache and permanent-output locations separate, with
none nested inside another or inside an input directory. Use a distinct report
name for every run rather than overwriting a prior report.

| Data | Behaviour |
| --- | --- |
| Local input directory/files | Read in place; the validator uses its own working copies when opening Excel. |
| Downloaded source ZIP | Retained in `cache_dir` across runs and reused by URL/content identity. |
| Extracted ZIP input | Owned temporary work, removed after the run when cleanup is confirmed. |
| Processor output with `{"kind":"temporary"}` | Uses owned working directories; not retained as a final workbook collection. Explicit processing caches have their own lifecycle. |
| Processor output with `{"kind":"directory","path":"D:/results/run-v1"}` | The final processing stage writes directly to that new directory. |
| Processor output with `{"kind":"archive","path":"D:/results/run-v1.zip"}` | Accepted final files are packaged into a new ZIP immediately after processing, before visual comparison. A later capture failure does not discard this requested ZIP. |
| Renderer working controls and internal validator caches | Kept beneath the current pair's owned work and removed after evidence publication; the standalone path does not implicitly retain XLSX controls in the legacy global caches. |
| Validation report | Retains scientific results, images, integrity manifests, provenance and the viewer independently of the processed workbooks. |

If processing-output archive publication fails, working results are preserved
for inspection/recovery. Unconfirmed process cleanup or failed evidence
packaging also preserves relevant working evidence, with its location in the
error, rather than deleting files still needed to understand the failure.

With temporary output and no persistent stage cache, the processed workbook
collection and internal renderer controls are removed after successful evidence
publication. The retained report remains viewable independently. Downloaded
original archives still follow their separate input-cache policy.

For direct A/B comparison, processing-output settings are not applicable; both
sources already exist. Their materialisation and the independent report are the
relevant lifecycles.

## Reuse completed processing stages

To persist processing results between runs, add these settings to a processing
configuration:

```json
"stage_cache_dir": "D:/validation-cache/stages",
"processing_cache_key": "processor-environment-v1"
```

The dependency key must be nonempty. Change it when external assumptions change,
including service/model versions, plugins or inherited environment not captured
as regular files. Declare script and configuration files in each stage's
`dependencies` array so their contents participate directly in its identity.
The coordinator also fingerprints the executable, stage configuration, input
workbooks and the relevant complete directory or original archive contents.

Reuse is at **whole-stage granularity**. A successful stage is reused only when
its identity matches and every cached output still has the recorded length and
SHA-256. Partial/failed stages are not published as reusable successes. A final
stage writing directly to an explicitly requested output directory bypasses this
stage cache and writes to that new destination; earlier stages can still reuse
cached results.

Persistent stage caching deliberately retains processed XLSX files, including a
final cached stage when final output is otherwise temporary. Leave
`stage_cache_dir` unset when those retained processing results are not wanted.
The download cache is independent: set `refresh_downloads: true` when the content
behind a URL should be downloaded again. Previously published download versions
are not silently changed beneath an active run.

This is reuse across new runs, not in-place resumption of a partially executed
report directory. Start a new report for a retry. Successful cached stages and
eligible prior visual observations can avoid repeated work.

## Reuse earlier visual observations under selected OOXML parts

Set a nonempty `environment_key` on the baseline run and keep the same key only
while its rendering assumptions still hold. Reuse also requires compatible
validator sources, capture policy/options, runtime capture assets, platform and
recorded dependency versions. A different signature rejects reuse rather than
assuming comparability. A report with a blank environment key cannot later be
made compatible merely by adding a different key to the new command.

For example, after the earlier `baseline-v1` run:

```powershell
.\target\release\sheet-scope.exe compare --a "D:\corpora\originals" --b "D:\results\shrink-v2" --report "D:\validation-reports\evaluation-v2" --reuse-from "D:\validation-reports\baseline-v1" --reuse-parts "worksheets" --environment-key "office-lab-2026-11-a"
```

Here SheetScope compares the selected worksheet member names and their
uncompressed content hashes for **both current A and current B against the
corresponding sides of the earlier observation**. It does not simply compare A
with B and infer that a previous result is reusable. Changed ZIP entry order,
compression or archive timestamps do not decide this check. Selected added or
removed parts do change it; XML is hashed as bytes, not semantically normalised.

Available CLI categories are `worksheets`, `workbook`, `styles`,
`shared-strings`, `relationships`, `drawings`, `charts` and `other`. Comma-separate
several categories. Omitting `--reuse-parts` selects all package members.
The Observatory exposes corresponding checkboxes; full JSON additionally permits
exact member names in `reuse_parts.extra_parts`.

For a worksheets-only JSON policy, explicitly disable the other categories:

```json
"reuse_parts": {
  "worksheets": true,
  "workbook": false,
  "styles": false,
  "shared_strings": false,
  "relationships": false,
  "drawings": false,
  "charts": false,
  "other": false,
  "extra_parts": []
}
```

Omitted boolean fields retain their default `true`; writing only
`"worksheets": true` does not select worksheets alone. Record all categories on
an initial baseline when practical, so a later run can choose a narrower subset.
Earlier fingerprints must cover the current selection; absent coverage triggers
a fresh comparison. Reuse requires an intact successful comparison package, and
copies its evidence into the new report so that report remains independent.

A restricted selection is an explicit scientific evaluation assumption. For
example, worksheets-only reuse does not inspect changed styles or images.
Reused entries are labelled as previous observations, not newly rendered or
proven visually equivalent workbooks. Failed or missing prior comparisons are
not reused as successes.

## Open or publish the report

Open a SheetScope report directory or its `run.json` without launching Excel or
re-running any processor:

```powershell
.\target\release\sheet-scope.exe open "D:\validation-reports\baseline-v1"
.\target\release\sheet-scope.exe open "D:\validation-reports\baseline-v1\run.json"
```

The report's generated `index.html` can also be opened directly in a browser.
Keep the complete report folder together: the overview, comparison pages,
shared local viewer assets and relative evidence paths are all needed.
The viewer includes paired images, difference pulse, pixel inspection, region
and chart navigation, coverage and recorded limitations.

Create a separate static website directory for sharing:

```powershell
.\target\release\sheet-scope.exe export "D:\validation-reports\baseline-v1" "D:\review-site\baseline-v1"
```

The export destination must be new. Copy that complete directory to a webserver,
or open its `index.html` locally. Reviewers need a browser only. Export retains
run metadata, eligible identity receipts and verified comparison packages; it
does not copy workbook working data, executables or processing/telemetry logs.
Those diagnostics remain in the original run report.

Opening interrupted evidence reconstructs an available index from durable
per-workbook records. It does not resume the run or mark unfinished work as
completed. A single legacy targeted `report.json` is not a SheetScope run index;
the standalone commands select a run directory or `run.json`.

## Current operating boundaries

Standalone workbook comparisons run sequentially. `max_processes` controls
external file processors, not parallel Excel comparisons. The existing targeted
engine retains its internal paired captures; `capture.serial_captures` can select
its diagnostic serial mode. Native Check Performance is disabled in this
standalone capture path.

The patch does not add non-ZIP archive readers, arbitrary one-to-many processor
output mappings, in-place restart of interrupted runs, a new tile-size control,
or a new hash-first rendering mode. The legacy optimized candidate selection,
layout/conditional-formatting analysis and chart checks remain the comparison
implementation.

For the precise contracts and further findings, see:

- [Sources, processors, subprocess outcomes and logs](validation-processing.md).
- [Portable evidence, the shared viewer and export behaviour](portable-reports.md).
- [Observatory resource behaviour and long-run investigation](observatory-long-runs.md).
