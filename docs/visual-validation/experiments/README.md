# Capture experiments

One reproducible protocol and one interface for individual experiments.

## Requirements

Windows desktop Excel, Windows PowerShell 5.1, Python 3.10+, Node.js for portable
tests, and .NET 8 SDK for the .NET host. Rust host/planner builds use the existing
Rust toolchain; WSL cross builds require MinGW-w64 and the Windows GNU target.
The native reference adapters require their recorded VBA/COM/add-in prerequisites.
No runner weakens macro security or trusts arbitrary certificates automatically.

The input registry and hashes are in [reproduce.json](reproduce.json). Place the
matching XLSX files at the listed paths below `corpus-local`, or pass `-CorpusRoot`.
The eight files are available from the `original_destatis_xlsx_files` source
repository identified by `regression/baselines/smoke.json`. Validate hashes rather
than substituting a newer portal file under the same name.

## Reproduce the decision

From the repository root in Windows PowerShell:

```powershell
& .\docs\visual-validation\experiments\reproduce.ps1 -PlanOnly
& .\docs\visual-validation\experiments\reproduce.ps1
```

The VS Code task **Visual validation: Reproduce capture decision** runs the same
entry. Close unrelated Excel instances and keep monitor/scaling fixed. The protocol
has **62 nominal cases**: 8 coverage, 36 host/overflow, 18 budget measurements.
Bounded recovery can add cases. The default session cap is six hours, not a promise
that all machines complete within six hours. Input/hash validation runs before Excel.

To run only a declared block, use `-Group tile-budget` (or `coverage`,
`host-overflow`). Results are scoped to the blocks actually executed. Repetition
count and timeouts can be changed explicitly; these changes enter the resolved plan.
They are not silently substituted into a claim about the default protocol.

## Individual experiments

**Experiment: visual validation** opens the WPF interface. The same route is:

```powershell
& .\docs\visual-validation\experiments\run.ps1 -InterfaceMode gui
```

The GUI offers all capture, drawing-bound, row-height and renderer diagnostics,
including the native bitmap/EMF and memory-pair references. For one selected capture:

```powershell
& .\docs\visual-validation\experiments\run.ps1 -InterfaceMode cli `
    -Experiment range-image -Workbook 'krankenhausverzeichnis-3500100227005.xlsx' `
    -Sheet ' KHV_2022' -HostKind dotnet -OverflowMode selective -TileMegapixels 32
```

Exact worksheet names preserve whitespace. `-SheetIndex` is one-based and uses the
original workbook order. `-ListSheets` reads only the workbook catalogue; hidden
worksheets are not unhidden. Use `-ListExperiments` for identifiers. Diagnostics
with synthetic fixtures require no corpus; the height-plateau diagnostic uses its
fixed hospital input. Native full-workbook adapters reject unsupported sheet subsets.

A JSON `-SelectionFile` avoids command-shell array ambiguity. It has schema
`vv-experiment-request-v1`, mode `single`, `experiment`, `workbook`, optional exact
`sheets`, `settings`, and `limits`. The GUI saves this request under `.tmp`, and the
resolved local report records all effective settings.

## Deadlines and continuation

| Limit | Default seconds |
|---|---:|
| Setup | 600 |
| Excel startup | 300 |
| Capture | 1200 (coverage uses 1800; comparison blocks 600) |
| Individual native call | 90 |
| Add-in inactivity | 120 |
| Postprocessing | 600 |
| Session | 21600 |

Use named `-StartupTimeoutSeconds`, `-CaptureTimeoutSeconds`,
`-NativeCallTimeoutSeconds`, `-PostprocessTimeoutSeconds` and
`-SessionTimeoutSeconds` overrides. Heartbeats never extend absolute deadlines.
The supervisor terminates only owned processes; unknown Excel ownership pauses work.

Explicit continuation: `reproduce.ps1 -ResumeRun '<managed run directory>'`.
Source and inputs must match. Completed cases are not silently rerun. Offline views
can be regenerated using `-ResumeRun ... -ReportOnly`; existing measurement times
are not edited. Add `-FullMetadata` only to request the full local textual evidence
instead of the default compact review projection. See [reporting](REPORTING.md).

## Validation

The portable validator also reports Git membership separately. `--audit-only` skips
code tests and writes the read-only findings to `.tmp/visual-validation/repository-audit.json`.
Required local inputs are not inferred to be tracked merely because they exist.


Run **Visual validation: Validate experiment code**, or
`python tools/validate-visual-experiments.py` for portable tests. The Windows task
also performs syntax, process-observer, snapshot I/O and local pixel preflights.
Portable tests and model renderers do not certify native Excel behavior.

[Protocol](PROTOCOL.md) · [Decision](../decisions.md) · [Published evidence](../evidence/index.html)
