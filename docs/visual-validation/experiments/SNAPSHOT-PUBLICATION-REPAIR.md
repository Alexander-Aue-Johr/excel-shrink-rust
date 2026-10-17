# Snapshot publication and infrastructure repair

## Incident: 20261017T132149838Z-2dd474cf

The uploaded focused run selected `kernhaushalt-long` / `csv-71717-22`, not the
remaining 26 worksheets. The supervisor failed after about 27 seconds while
reading `capture-lifecycle.json`: `CreateFileW`, Win32 error **32**. No image was
received, no measurement-start milestone was recorded and no owned Excel process
receipt was present. All twelve local web assets passed preflight. The preserved
file later contains valid JSON; that does not contradict an earlier sharing conflict.

Win32 32 is a sharing violation, not MAX_PATH and not a PNG limit. The particular
competing handle was not identified. The old shared JSON writer was nevertheless
unsafe: any `File.Replace` failure invoked a copy-over fallback that could lock or
partially overwrite the live destination while another process read it.

## Changes

`SnapshotPublication.cs` supplies a small package-free Windows helper compiled by
Windows PowerShell and the ASP.NET host. JSON is serialized before IO, staged to a
unique sibling, flushed and closed, then published using `MoveFileExW` with
`REPLACE_EXISTING | WRITE_THROUGH`. There is no copy/delete fallback on the live
file and no cross-volume copy flag. The normal reader shares read/write/delete.

Readers retry only sharing/lock errors 32/33. Publication additionally retries
error 5, which can occur during replacement, but never changes ACLs or treats a
persistent permissions failure as success. There are seven attempts with six
waits (20, 40, 80, 160, 320, 500 ms). The waits total 1.12 seconds; filesystem-call
latency is additional. Missing files are distinct from malformed/empty JSON.

On failed publication, the previous snapshot is not explicitly overwritten or
deleted, and the closed staged file is retained. A `VV_SNAPSHOT_IO` exception names
operation, native error, attempt count, destination and staged-file completeness.
The supervisor reports such failures as infrastructure/report IO, not a renderer
failure. Original failures stay in their historical reports.

The .NET host now marks a workbook as published only after its output JSON has
been published. Its `/status` completion flag no longer depends merely on receiving
an in-memory result. This reduces the race in which the runner could close Excel
before final result publication. The existing output gates and timeouts remain.

No universal atomicity/power-loss guarantee is asserted for arbitrary network
filesystems. A filesystem that cannot perform the required replacement will fail
the native preflight rather than fall back to rewriting the live file.

## Two related integration corrections

* `/config` exposes `host_run_id` for all experiment families. Asset preflight
  validates that launcher-generated identity, experiment and host. Capture and
  diagnostic write tokens remain separate and retain their existing validation.
  Shape/renderer diagnostics no longer fail because .NET legitimately has no
  `capture_run_id` for those experiment families.
* `decision-campaign.ps1` without script arguments prints entry choices and
  returns before configuration loading, WSL or Excel. The historical campaign
  requires `-Campaign`. Its existing VS Code task passes that flag explicitly.
  Focused and final-qualification commands retain their existing flags.

These corrections do not change capture pixels, bounds, calibration, context,
overflow, tile budgets, PNG compression or the Rust telemetry implementation.
They do not repair/re-run the historical VBA/GDI+ reference experiments.

## Native preflight (no Excel)

Focused runs execute `tests/snapshot-publication.tests.ps1` before opening any
workbook. The probe runs on the report filesystem, not just local `%TEMP%`, and
records `snapshot-preflight.json`. It tests create/read, old-reader consistency,
short read and rename conflicts, a permanent conflict retaining old and staged
versions, invalid JSON, a second PowerShell writer with a concurrent polling
reader, and experiment-family identities. Only its private scratch directory is
removed after success. Failed probe evidence remains.

Standalone execution from the repository root:

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass `
    -File .\docs\visual-validation\experiments\tests\snapshot-publication.tests.ps1 `
    -ProbeRoot .\docs\visual-validation\results
```

These are Windows filesystem tests, not an Excel/Office test. A failed probe does
not establish anything about the quality of a workbook's screenshots.

## Next capture: only the missing Kernhaushalt tail

The package includes an exact request, so no manually copied JSON is necessary:

```powershell
powershell.exe -STA -NoLogo -NoProfile -ExecutionPolicy Bypass `
    -File .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused `
    -SelectionFile .\docs\visual-validation\experiments\capture-selections\kernhaushalt-remaining.json
```

This selects `csv-71717-23` through `csv-71717-48` (26 names), .NET / Selective /
16 MP, a 300-second start budget, 1800-second capture budget and 3600-second session.
`csv-71717-22` and earlier worksheets are not recaptured. The native preflight above
runs automatically. Append `-PlanOnly` to inspect the selection without Excel.

Submit the new run's `review-metadata.zip`, including `snapshot-preflight.json`.
Do not rerun the broad campaign to test a shared snapshot publication repair.

## Validation boundary

Python source-contract and controller tests plus existing JavaScript tests run in
the patch workspace. Windows PowerShell 5.1, Win32 filesystem calls, .NET/Rust host
builds and Excel are not available there. The native preflight is supplied for
execution on the target Windows/WSL filesystem; it is not claimed to have run in
the patch workspace. No successful new Kernhaushalt capture is claimed.

Primary API references:
- Microsoft System Error Codes (0–499): https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-
- Microsoft CreateFileW: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
- Microsoft MoveFileExW: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
