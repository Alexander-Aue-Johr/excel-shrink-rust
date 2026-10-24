# Resident cleanup isolation investigation

The current cleanup barrier is a conservative policy, not proof that closing
one Excel instance necessarily disrupts another. It retains completed groups
until all peers in the wave are ready, including their Excel memory.

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
on a clean desktop or in all startup/ownership orders. The barrier remains in
place pending that test. No corpus result is invalidated or recomputed here.
