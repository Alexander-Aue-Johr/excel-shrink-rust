# Resident cleanup isolation investigation

Interactive control-room groups now release their own cleanup gate as soon as
they finish. Completed groups no longer retain Excel memory for slower peers.
Set `EXCEL_SHRINK_SHARED_CLEANUP=1` before starting the runner to restore the
shared gate for diagnosis. Fixed noninteractive batches retain their old policy.

## Actual process operations

`ResidentProcesses.Start` assigns each Excel process to a per-launcher Windows
Job Object before resuming it. Children normally inherit membership. The job
uses `KILL_ON_JOB_CLOSE | JOB_MEMORY`. `resident.ps1` disposes this job on group
cleanup, so cleanup terminates the job's descendants, not just workbook windows.
`ResidentProcesses.Stop`, used for an individual reset, calls `TerminateProcess`
on the owned process handle; it does not enumerate and kill other Excel PIDs.

This establishes what cleanup terminates. It does not establish that a peer
uses the terminated WebView processes. No reproducible original failure trace
was identified during this investigation.

## Bounded counterexample, 2026-10-24

Two independent resident launchers each opened a small workbook and returned
successful Office.js captures. Session A then completed and disposed its job.
Session B successfully captured again, with identical dimensions and SHA-256.
The valid experiment passed twice; the checked-in test was then strengthened
to require successful A launcher exit and complete cleanup metrics as well.

Run explicitly (opens two hidden Excel instances):

```powershell
python excel-shrink-cli/office-capture/tests/cleanup-peer-smoke.py --run
```

Receipts and launcher logs stay under `.tmp/cleanup-peer-check/<timestamp>/`.
The test never sends commands to the production corpus.

**Limitation:** other corpus Excel processes were running during these checks.
They might keep a shared Office runtime alive. This counterexample disproves
an unconditional claim, but does not establish safe independent group cleanup
on a clean desktop or in all startup/ownership orders. Independent cleanup is
therefore observed during a bounded four-group small-files-first corpus trial.
No completed corpus result is invalidated or recomputed by this policy change.

## Correlating a future peer failure

Each group writes `cleanup-<attempt_id>.json` in its control-room directory.
It records cleanup start/completion UTC epoch milliseconds and job members as
`pid|process name|UTC process start`. The snapshot queries actual Windows Job
membership; it does not infer ownership from names or parent PID. The existing
launcher event stream also records `owned_cleanup_started/completed`.

Compare these timestamps with peer `client_error`, process exit, config/add-in
reconnection and failed capture events. Fresh heartbeats alone are not progress.
Temporal proximity is a suspicion, not causal proof. Reproduce the same peer
capture with and without the other group's cleanup; retain both attempt IDs.
Failed or missing capture responses remain failures, never equality evidence.

## Clean-desktop failure and profile isolation

With the corpus stopped, the original shared-profile launcher reproduced
`TASKPANE_RESTARTED` in B immediately after A disposed its job. This is stronger
evidence than the earlier passing checks alongside a running corpus.

Each resident launcher now sets `WEBVIEW2_USER_DATA_FOLDER` to its own staging
`webview2-profile` before creating Excel. The same two-group test then passed;
a four-group test also passed all three sequential cleanup/capture checks.
Run that bounded check with `--run --groups 4`. Receipts preserve failures as
well as successes. This isolation is deliberately per group, not per workbook
variant: variants remain alive together until their group is finished.
Independent control-room groups publish their own cleanup release after finishing;
closing their owned Excel job does not require a scheduler polling round trip.
The scheduler batches derived full JSON/HTML report refreshes (at most once per
30 seconds, when its event queue is idle). Durable per-workbook records remain
immediate. This prevents rebuilding the entire report for every progress event
from delaying completion handling and replacement admission.

## Four-pane reset and upload checks (2026-10-25)

The expanded check uses two groups with four variants each, resets one ready
pane in each group before assigning work, and verifies the actual WebView
profile paths in process command lines. It then starts image uploads in the
surviving group while closing the other group, compares those images and the
subsequent captures, repeats image-first batches, and releases retained plans:

```powershell
python excel-shrink-cli/office-capture/tests/cleanup-peer-smoke.py --run --groups 2 --clients 4 --reset-first --image-batches 6
```

Both the small-sheet reset check and the populated 1,000-row, eight-column
upload check passed. All surviving variants retained identical PNG hashes.
The local upload receipt is `.tmp/cleanup-peer-check/1792932733119669600/receipt.json`.
Its six repeated batches had zero pending PNG characters after each completion;
retained plan counts stayed at two and explicit release cleared plans, tiles,
and cached geometry. The four panes' nine owned WebView processes ranged from
503 to 598 MiB private bytes; JavaScript heaps showed garbage-collection drops.
This bounded check found no accumulating retained-image evidence; it is not a
long-duration leak guarantee. Those checks opened their groups sequentially;
they did not establish reliability during concurrent startup of real files.

New sessions record document/run/request IDs. Repeating configuration in the
same document/run is idempotent; a different document after work started produces
`TASKPANE_CONTEXT_CHANGED` and invalidates that attempt. This observation alone
does not prove a renderer crash. A bounded latest pane-resource snapshot and
per-process PID/birth/memory snapshots support correlation with cleanup events.
The native startup measurement is described in [native-readiness.md](native-readiness.md).

A separate bounded `CloseMainWindow` trial requested normal closure for all four
retained Excel processes per group, with at most one shared second before the
existing Job cleanup. In both hidden groups, zero requests were accepted and
zero processes exited through that path (17 and 12 ms spent making requests).
Peer uploads and later captures still passed. The local receipt is
`.tmp/cleanup-peer-check/1792933061343227800/receipt.json`; each group's
`metrics.json` retains the trial's `graceful_close` counts. Since this provided
no successful graceful closure for the actual hidden runner, production keeps
the known-owned Job cleanup. This result does not establish whether visible
Excel windows can close gracefully; that separate behavior remains unproven.

## Concurrent hidden startup and private desktops

The four-group corpus restart exposed a different failure before any capture:
the `maximum-cleaning.xlsx` windows existed, but their add-ins never requested
configuration. Other panes continued sending heartbeats. The native windows
were responsive, memory was low, and no modal dialog was found. Private copies
of the affected WV, DC and WY AP/IB workbooks reproduced this in bounded trials:

| Startup policy | Observation |
| --- | --- |
| Two concurrent groups, hidden windows, group browser profile | One maximum-cleaning pane timed out. |
| Same inputs, separate profile per Excel process | Startup still timed out; profile sharing was insufficient to explain it. |
| Same inputs, visible windows | All eight panes and captures completed. |
| Minimized startup, hide after readiness | Captures completed, but Excel took keyboard focus despite the non-activation flag. |
| Normal rendering on a private Windows desktop | Hidden groups initialized and captured without appearing on the user's input desktop. |
| Four private-desktop groups, then one explicitly visible reset | Completed after separating the private and interactive browser profiles. |

Hidden resident sessions therefore reuse `CaptureDesktop`, already used by the
classic capture runner. Excel renders normally on that private desktop; it is
still assigned to its owning Job before its first instruction. The launcher
does not hide or minimize these private windows. Every group remains free to
start independently. Browser profiles are separate per group **and desktop**;
a visible hard reset cannot reuse a browser on the private desktop.

Client metrics retain `window_mode` and the actual configured profile directory.
Readiness and memory sampling still precede dispatch. The existing hard-reset
rules and per-group cleanup ownership remain unchanged.

Run the concurrent startup check against copies of resident client specifications
and their corresponding staged inputs:

```powershell
python excel-shrink-cli/office-capture/tests/concurrent-startup.py --run --cases .tmp/profile-concurrency-inputs --groups 4 --reset-visible
```

The optional reset deliberately shows one Excel instance. All other test Excel
processes must stay out of sampled foreground focus. The test publishes a receipt
with each group's final startup metrics, focus observations and any failure.
Local experiment receipts are under `.tmp/profile-concurrency-results/`:
`1792933860530778300` (shared-profile failure), `1792933949105173400`
(per-process-profile failure), `1792934302066802600` (private desktop pass),
and `1792934529662463700` (four groups plus visible reset pass).
