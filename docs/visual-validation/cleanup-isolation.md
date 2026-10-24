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
