# Compact validation diagnostics

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
