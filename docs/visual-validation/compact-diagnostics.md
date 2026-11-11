# Compact validation diagnostics

## SQLite diagnostics in corpus launches

Corpus launcher settings default to `sqlite_metadata=true`. The launch plan sets
`EXCEL_SHRINK_SQLITE_METADATA=1` for comparison snapshots and resident diagnostic
events. Each writer owns its database. Completion exports remain compatible with
existing report readers. `settings.sqlite_metadata=false` selects the legacy
diagnostic writer. Command/response IPC files and scientific result JSON remain
separate from these diagnostic databases.

The native chart experiment exercised both chart modes and a changed-input case
with SQLite enabled. All nine resulting databases passed `integrity_check`.

`GET /api/summary?limit=8&stalled_after_seconds=120` returns bounded current-run
comparison evidence, current and last successful host operations, process and
Office.js liveness separately, queue counts and available throughput estimates.
It exposes execution strategy, capture policy and report locations so an operator
can verify which comparison is running. Long operations are inspection hints,
not automatic failures or proof of a deadlock. Full evidence remains in reports.

The existing Observatory control endpoints remain responsible for configuration,
group restart and command receipts. This projection adds no second control path.

For quiet local monitoring, run the read-only watcher in a separate terminal:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/watch-validation.ps1 -Observatory http://127.0.0.1:8771 -OutputDirectory .tmp/validation-watch
```

It updates `latest.json`, records only changed attention/errors and rotates alert
files at 16 MiB. `-Once` prints a single snapshot. The watcher consumes no model
API calls. Diagnostic alerts require inspection of the associated operation and
retained evidence before deciding to retry or change behavior.

A host that has completed but still occupies a current `waiting_cleanup`
scheduler slot is reported as `completed_host_waiting_for_scheduler` after the
configured observation interval. Such entries remain prioritized in the bounded
summary. A historical completed attempt does not trigger this observation for a
newer replacement.

## Inspect CF and layout selection without Excel

```powershell
excel-shrink-cli.exe regression inspect-candidates --left before.xlsx --right after.xlsx --sample-limit 8
```

The command uses the live pipeline's XML snapshots and CF dependency selector.
It reports reason counts, potential cell-input changes and the count of logical
CF regions, with bounded decision/region examples. `--sample-limit` accepts 0–100.
The `layout` section reuses the live layout selector and reports its logical
region count, selected-sheet count, shared layout-context changes and bounded
sheet examples. Each example identifies changed sheet defaults or unmodeled
layout metadata, the visible-content envelope and background-tail sampling.
Both region and sheet examples respect the sample limit, including zero.
Logical layout ranges are reported before grid expansion, so a range ending
at XFD can be investigated without allocating its capture tiles.
It leaves the input workbooks unchanged and releases its temporary fingerprints.
It does not open Excel, measure effective heights, capture charts or issue a
visual verdict. The reported logical regions precede live geometry partitioning.

### Rebuild stage telemetry

Resident heartbeat resource samples include bounded `targeted_progress` evidence with command ID, stage and transition timestamp. Rebuild dispatch, waiting for calculation, saving and successful save are distinct stages. A heartbeat alone does not prove calculation progress. Command deadlines and save/rebuild success requirements are unchanged.
