# Corpus launcher on Windows

Run the VS Code task **Excel Shrink: Configure corpus run**. It opens the
same kind of PowerShell/WPF dialog used by the existing workbook demo tasks.
The dialog configures the existing `regression study` runner; it does not create
a separate results format or a separate comparison engine.

In a WSL workspace the task uses `tools/start-corpus-wsl.sh` and
`tools/start-corpus-wsl.ps1` to prepare a Windows working copy under
`%LOCALAPPDATA%\excel-shrink-rust\wsl-workspaces\<workspace-id>\repo`.
Windows file locks are unsupported on WSL UNC paths, so builds are deployed
and runs write their results in this local Windows copy. Tracked source edits
are synchronized each time the task opens; an active Windows run prevents sync.

On first use, existing results are copied and verified with SHA-256. Originals
remain in place, differing destination files are never overwritten, and the
source run must be idle. Existing ZIP bindings retain their original paths;
large WSL archives are read directly rather than copied. New downloads and
future results are stored in the Windows working copy. Its launcher settings
persist independently of the WSL configuration.

Release builds use the WSL Rust toolchain with the
`x86_64-pc-windows-gnu` target and MinGW linker, then copy the executable to the
Windows working copy. The runner locates capture scripts beside its deployed
workspace rather than using the Linux path embedded by the compiler. Windows
Git and the .NET 8 SDK must be available; a separate Windows Rust installation
is not required. The launcher also discovers existing private .NET 8 SDKs in
the capture toolchain cache and restores the control host before starting it.

`bash tools/start-corpus-wsl.sh -PrepareOnly` prepares the working copy without
building or starting a run. Other arguments, such as `-Preview` or `-Headless`,
are forwarded to the Windows launcher. A lock-check failure is not evidence of
an active run.

Select a corpus, one size partition or the whole corpus, and the processing
order. Partitions contain approximately equal **numbers of files** in the
runner's deterministic size order. Selection uses the complete source catalog,
so previously completed work does not move partition boundaries. Processing
order is independent: the largest quarter can still run smallest first.

The group count includes **0**. A new run with zero groups starts paused; raising
the count in the Observatory resumes work. Reducing an active run to zero stops
admission of new work while current work reaches its normal completion boundary.
The launcher does not terminate Excel to apply a change.

Very tall sheets now use a targeted capture selection by default. The dialog
exposes two positive integers: **more than 40 capture rows** triggers selection;
**2 capture rows at each end** keeps the beginning and end. Areas selected by
XML, recalculation, geometry and visual-content indicators are added. This is
not a cap on required captures. Smaller sheets retain complete capture, and
omitted regions are explicitly marked as not visually checked.

The same settings are available under **Capture & opening** in the Observatory
and apply to the next workbook group. Existing launcher configurations acquire
the defaults without changing archive bindings or completed results. The
threshold counts actual vertical capture bands, so the capture pixel budget
also affects when it is reached.

With the Observatory enabled, this count is the manually controlled target for
concurrent workbook groups. Memory limits remain safety limits; they do not
silently replace the target with the adaptive admission mode used by older CLI
examples without the control room.

Advanced settings expose the output and download directories, Observatory port,
memory caps, timeouts, release build, and result-package imports. Initial memory
caps are conservative on small machines. Explicit limits from this machine's
existing launch metadata are retained. A copied configuration resets machine
memory limits and group count to local defaults before use.

The release build starts only after the launcher checks that the result directory
is idle. A second start against an active directory is rejected with its
Observatory address. The long run starts as a separate hidden process with its
own stdout/stderr files; closing the dialog or the VS Code task does not issue a
stop command. Excel visibility remains a separate checkbox.

## Existing archives and results

`tools/corpus-launcher/catalog.json` contains the existing Enron, Destatis and
Data.gov identities and expected archive names. Private download URLs, local
paths, settings and additional corpora are stored in the ignored file
`.tmp/corpus-launcher/config.json`.

When that local configuration does not exist, the launcher reads:

- `.tmp/archive-regression/study-request.json` for original corpus identities and
  download URLs;
- `.tmp/distributed-study-request.json` for local ZIP bindings;
- `.tmp/archive-regression/tower-active-run.json` for the existing output,
  approved preparation builds and matching machine settings.

The migration binds existing ZIPs **in place**. It does not rename, move, copy or
download them. Existing output remains `.tmp/regression/study` unless explicitly
changed. The runtime request retains every corpus, even when `--run-corpus`
selects only one. Workbook identities, prepared artifacts, completed records,
source-catalog checks and portable result imports continue to use the existing
runner mechanisms.

Existing IDs never depend on a renamed ZIP path. **Link local ZIP** checks
the expected workbook count. When replacing an existing binding, it also checks
SHA-256 against the previous archive or its recorded hash. This occasional
rebind can take time on a large ZIP; ordinary launches do not hash it again for
the launcher. If the old ZIP and its verified hash are both missing while the
output already contains records, rebinding is refused. Restore the original
archive first. On a fresh machine without historical results, supply the original
archive to establish the initial binding and hash.

New remote corpora use a stable ID derived from the HTTP(S) URL. The URL can be a
download endpoint or signed URL; the runner validates the downloaded ZIP. New
local corpora receive a persistent ID in the local configuration. Share that
corpus entry, preserving its ID, when distributing the same new local corpus to
another machine. Do not register it independently under a new identity.

The recorded preparation-build allowlist is preserved. The launcher never
automatically approves the previous executable hash for a newly built runner.
After reviewing that preparation behavior is compatible, add an explicitly
approved hash with `-ApprovePreparedFrom`. No change to download naming or the
UI itself requires existing results to be deleted or rebuilt.

The previous local wrappers remain available for historical reproducibility,
but the VS Code task no longer depends on any corpus-specific launcher:

- `.tmp/resident-capture-trial/start-datagov.ps1`
- `.tmp/distributed-setup/start-datagov.ps1`
- `.tmp/whitespace-followup/start-study.ps1`
- `.tmp/stability-review/start-reviewed-study.ps1`

The former **Release corpus regression (resume)** task is replaced by the WPF
task. Existing direct Rust CLI commands remain supported.

## Preview, migration and terminal use

From the repository root, inspect the resolved bindings without saving or
starting anything:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-corpus.ps1 -Preview
```

Save only the local configuration, retaining the same archives and results:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-corpus.ps1 -MigrateOnly
```

For a reviewed preparation-compatible build, append
`-ApprovePreparedFrom <64-character-sha256>` to the migration command. This is an
explicit compatibility decision, not an option needed for normal launches.

Run the same saved configuration without the dialog:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-corpus.ps1 -Headless -Corpus Data.gov -Partition 0 -Order size-desc -Groups 3
```

Choose one Enron quarter and start with its smallest files:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-corpus.ps1 -Headless -Corpus Enron -Partitions 4 -Partition 1 -Order size-asc -Groups 1 -HideExcel
```

`-Partition 0` selects the whole corpus. `-Groups 0` starts paused. `-SkipBuild`
uses the existing release executable. Overrides persist with a successful
configuration save/start; preview is read-only. `-ConfigurationPath` supports
separate machine configurations and `-ImportResults` accepts result ZIP paths.

`-TallSheetThreshold 75 -TallSheetEdgeTiles 3` changes the tall-sheet threshold
and keeps three capture rows at **each** end. The launcher passes these through
`EXCEL_SHRINK_TALL_SHEET_THRESHOLD` and `EXCEL_SHRINK_TALL_SHEET_EDGE_TILES`;
the launch receipt retains their values. The Observatory can update them for
subsequent groups during the run.

## Reports, transfer and diagnostics

The Observatory starts before existing results and ZIP catalogs are read. During
startup it displays **Preparation**, the current corpus and phase, and a
measured entry counter and progress bar where the phase has a known total.
Counters refer to the current phase, not the whole launch: archive enumeration,
catalog checks and partition selection are separate passes. Opening a ZIP,
imports and run initialization show an indeterminate bar instead of an invented
percentage or ETA. The normal workbook totals replace this view after startup.
The server itself may need a short initial host build before its URL responds.
Previously started runners retain their existing startup behavior.

Each Excel process card includes **Excel Hard Reset** and the available file
size, measured opening time, opening memory, and Shrink duration. Profile size,
opening time, and memory show signed differences against the original. Missing
measurements remain explicitly unknown; opening times come from the separate
regression measurement, rather than the add-in startup wait.

A hard reset stops only that owned Excel process and reloads a fresh source copy
using the current hidden/visible setting. Before any comparison command has been
dispatched, its peers remain open. Once commands have started, the process is
stopped and the group is requeued to discard the previous attempt's evidence.
Closed groups and older launchers also use the group retry. Completed files and
prepared Shrink outputs remain reusable. Requests are bound to the displayed
attempt and the launcher's exact process identity.

The isolated native check supports this path:

```powershell
python excel-shrink-cli/office-capture/tests/tall-sheet-smoke.py --run --hard-reset-original --port 8803
```

The runner still writes its usual report and records under the selected output
directory. With the export checkbox enabled, it writes a transfer ZIP under
`<output>/exports/` after finishing the selected work. The filename includes
machine, corpus, partition and launch time. Import packages through the dialog
or the existing CLI; their source IDs and provenance remain intact.

Each launch writes a receipt, request, stdout and stderr under
`.tmp/corpus-launcher/runs/<time-id>/`. The receipt records PID, process start,
commit, executable SHA-256, exact arguments, memory environment and export path.
The latest receipt also appears at `.tmp/corpus-launcher/last-run.json` and
`<output>/launch.json`. Private request/configuration files are local artifacts;
do not publish them as part of a paper artifact without reviewing their URLs.

An obsolete idle Observatory host belonging to this exact output may be
refreshed on start. A foreign service or another output on the configured port
is left running and causes a clear port-conflict error.

## Isolated checks

```powershell
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File tools/corpus-launcher/Tests.ps1 -NativeArgumentTest -DialogTest
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/corpus-launcher/Wsl.Tests.ps1
```

These checks use synthetic ZIPs and a short-lived fake runner under
`.tmp/corpus-launcher-tests/`. They exercise migration, identity preservation,
rebind rejection, partition/order controls, group count zero, output locking,
Windows argument quoting, detached process metadata and environment restoration.
No real corpus is downloaded or processed, and no Excel process is started.
The WSL migration checks use temporary Windows files to verify copying,
original retention, repeatability, lock exclusion, conflict rejection and
rejection of an active source output.

Excel diagnostic logs under `%LOCALAPPDATA%\Temp\Diagnostics\EXCEL` are pruned
by a hidden helper every 30 seconds while the exact runner PID/start time lives.
Only `.log` files older than 60 seconds are removed; locked files and reparse
points are skipped. The launch receipt contains `ExcelDiagnosticsCleanup` and
the run directory's `excel-diagnostics-cleanup.json` records the last pass.
Reports and corpus artefacts are outside this cleanup root. Manual one-off:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/clear-excel-diagnostics.ps1 -Once
```
