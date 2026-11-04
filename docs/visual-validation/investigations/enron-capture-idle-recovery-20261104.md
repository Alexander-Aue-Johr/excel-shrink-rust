# Enron recovery probes and capture-idle restart, 4 November 2026

Machine: THE_LIBRARY, AMD Ryzen AI 9 HX 370 / Radeon 890M, 32 GiB installed
(29.62 GiB usable), Windows 11 Pro build 26300. Corpus output:
.tmp/regression/enron-upper-20261102, the larger original-size half (2/2).

## Changes and observed causes

- Case 08f7718c6c1f7e37e2e8 reproduced an opening timeout. Excel was alive with
  zero add-in heartbeats. Read-only UI Automation on its private desktop found
  Excel's content-recovery prompt. Owned repair dialogs now fail explicitly
  with opening-dialog.json; no repair is accepted automatically.
- Case 07adad92f7a5866ae2aa previously exceeded 250,000 height requests.
  Column/style changes had selected through row 1,048,576. Selection now checks
  all rows above/within the union of content envelopes plus margin, retaining
  identical-fixed-row exclusions. The formatting-only tail below that envelope
  is excluded explicitly, never certified equal.
- Capture plans probed split boundaries through repeated native round trips.
  Balanced cell-coordinate bisection measures each child once. Exact selected
  cell coverage and per-state image budgets remain required; hidden zero-area
  leaves remain explicit without image exports.
- The targeted caller used a blocking process wait without its own watchdog.
  The final policy is capture inactivity, not a wall-clock workbook cap:
  validated, persisted cell/chart images advance capture_success_sequence.
  Both launcher and independent parent observe it; the parent allows 15 seconds
  cleanup grace. Heartbeats, phase changes and lane resets do not count.
  Continuous captures may run indefinitely. The run uses 120 seconds without
  a successful capture, including before the first image.
- Timeouts retain a failure receipt and release the owned process job. Corpus
  errors preserve the concrete native reason rather than only its directory.

## Controlled validation

Local evidence: .tmp/performance-audit/recovery-20261104/.
The final one-/two-group experiment uses round3-idle after the user requested
capture-reset semantics. See matched-verdicts-idle.json for its comparison.

The same three earlier failing inputs are checked sequentially, then with at
most two groups. Original and prepared Default files are restored from existing
ZIP bundles and checksum-verified; no conversion or shrinking is run.
The repair-prompt case is an expected input failure in both modes.
The other two cases must complete with bounded targeted coverage.

Final capture-idle results (one / two groups, seconds):

| Earlier failure | One group | Two groups | Result |
| --- | ---: | ---: | --- |
| Opening / repair prompt | 8.643 | 10.280 | Same explicit repair-required failure |
| Resident exited | 41.327 | 46.017 | Completed with limitations in both |
| Height request budget | 30.273 | 32.159 | Completed with limitations in both |

The completed pairs had exactly matching candidate counts, difference flags
and measured geometries. The successful one-group cases published 84 and 51
persisted-image receipts; the repair case published zero. Final phase windows
contained one Windows Workload application crash during the one-group phase
and none during the shorter two-group phase; no Excel/WebView application
crash was recorded in either. This is a small sample, not a long-run stability
proof. Portable observations: [JSON](enron-capture-idle-recovery-20261104.json).
Earlier round2 results, before the inactivity refinement, were:
repair required 8.691 / 9.595 seconds; earlier Resident exit 38.788 / 41.945;
earlier height-budget error 28.475 / 30.018. Verdicts and geometries matched.

Supplementary large input 87c8ac003fd17e1f7abe was intentionally stopped at
the then-configured 240-second total test limit after completing 304 of 385
Original capture regions. This established real capture workload, not an idle
freeze, and was not a successful complete comparison. The fixed total-time
policy was subsequently replaced; it is not the production policy.

Automated checks: targeted Rust suite 108 passed / 10 ignored; corpus launcher
90 checks; synthetic relative-reduction/tie-break ordering; owned repair-prompt
classification; a heartbeat-only child is stopped while an image-receipt child
runs beyond its one-second test idle limit; unrelated peer processes survive.
The Rust clock test extends a short idle limit over 99 successive captures.

Windows WorkloadsSessionHost.exe / AMD onnxruntime_vitisai_ep.dll crashes
remain unresolved. Event windows differ in duration; counts must not be read as
a concurrency-rate comparison. These probes do not establish that the earlier
desktop-freeze cause is fixed.

## Restart configuration

Existing 10,110 prepared bundles are retained. The run uses --compare-only,
Original versus Default before/after rebuilding the same SD instance, CP off,
two comparison groups, and a 4 GiB system reserve. Producer approval remains
pinned to the existing unchanged shrink preparations.

tools/order-prepared-by-reduction.ps1 reads ZIP directory sizes and orders
relative Default reduction descending, then saved bytes and identity. It writes
an explicit study workbook list plus CSV. Partition membership stays unchanged.
The run list is:
.tmp/enron-full-hash-screening/reduction-priority-20261104/workbook-list.json.
It ranks 10,110 preparations; the first two save about 99.33% and 99.11%.
The corpus launcher persists this path as workbook_list.

Visual success remains completed_with_limitations, not whole-workbook equality.
Invalid inputs, capture-idle expiry and incomplete coverage remain failures.
A later invocation can retry them.