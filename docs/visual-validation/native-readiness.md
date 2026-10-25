# Native startup measurements and bounded diagnostics

The visual corpus no longer opens workbooks in COM solely to measure opening
time or memory. Historical COM `Workbooks.Open` observations retain their old
method and provenance; they are not mixed with the native observations below.
Legacy `.xls` conversion still requires COM.

## What the native measurement means

`launch-to-addin-ready-v1` starts immediately before creating the native Excel
process. It ends when the add-in successfully reads the expected workbook name
through the Excel API and reports readiness to its host. It therefore includes
Excel startup, workbook opening, taskpane initialization and local communication.
The launcher samples the retained Excel process's private bytes and working set
once at this boundary, before capture commands are admitted. These are two memory
measures of Excel, not an all-descendant sum or a workbook-only allocation.

Snapshots retain PID, process creation time, physical client, run/machine and
attempt. Identical profile files can share a physical client. Such aliases are
explicit and do not create independent speedup observations. A classic fallback
without a verified native sample displays no new opening measurement.

## Recalculation remains separate from capture

Run the opt-in saved-value parity check with:

```powershell
python excel-shrink-cli/office-capture/tests/recalc-parity-smoke.py --run
```

On 2026-10-25, Office.js `FullRebuild` followed by `Workbook.save('Save')` agreed
with the existing COM recalculation for deterministic cached values in both
manual and automatic calculation fixtures. The fixtures include cross-sheet and
hidden-cell references, string/error formulas, a chart and conditional formatting.
RAND/NOW cells are explicitly excluded from deterministic equality, and their
pre-recalc observations remain in the receipt. Automatic opening had already
recalculated stale formula caches before the add-in ran; manual opening had not.

This is evidence of API feasibility, not complete replacement validation. The
probe does not establish equivalent handling of external-link prompts, macro
security, protected files or the full corpus. COM can configure those opening
controls before opening the target; an add-in starts later. Production saved
recalculation references therefore retain the existing COM path for now. Capture
inputs remain separate and are never saved by that probe or by production workers.

The test stages a private copy of the native launcher/web assets and adds only a
test recalculation hook. It never adds a saving command to the production worker.

## Observations are not crash diagnoses

A repeated config request alone is not proof of a WebView crash. Runtime/run
identities distinguish idempotent repeated requests from a replacement context.
Changing the context after work was dispatched invalidates that attempt rather
than merging evidence from two contexts. Browser process identity and memory
samples, job/heartbeat history and cleanup timestamps support later correlation.

Pane telemetry keeps one current bounded snapshot: retained plans/tiles,
geometry entries, pending image bytes/pixels and available JS heap counters.
These are diagnostic counters, not a heap dump. In-flight network operations
have a bounded timeout; Office operations retain the runner's separate deadline.
A high temporary image allocation is not by itself a leak. Compare idle memory
after repeated batches and release, and distinguish JavaScript heap from native
WebView memory. Full-tile PNG forwarding avoids an unnecessary decoded bitmap.

Uncertain Windows cleanup retains the affected group's scheduler slot and files,
reports the error and retries at 30-second intervals. Peers can continue. An
unverified process never causes repeated admission of replacement groups or an
unrelated PID to be terminated.
