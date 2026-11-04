# Excel opening stalls, 4 November 2026

## Scope and environment

Machine: THE_LIBRARY, AMD Ryzen AI 9 HX 370 w/ Radeon 890M, 24 logical CPUs, 32 GiB installed RAM.
Windows 10.0.26300 / build 26300; Excel 16.0.20527.20112.
The corpus runner was stopped cleanly before the probes. Inputs came from the
backup of run `b5373dd3-c4ad-4072-9e0e-46181de5c444`, output
`enron-upper-20261104-shrink-51b0905`. Its original records and images were not changed.

All 26 records containing `No successful capture for 120 seconds` were inspected:
18 had no successful captures; seven stalled on the Default rebuild; one stalled
on post-rebuild range measurement after 1,036 captures. A failed Excel lane alone
does not establish an opening failure: cleanup also closes lanes after a timeout.

## Reproduction

Original and Default were each opened separately on owned private desktops,
with readiness, native windows and dialog observations retained. Automatic-open
probes used a 20-second observation window; this is not proof that a file can
never open. Two UI Automation observation errors were rerun successfully and
are excluded from final classifications.

Automatic startup: 26/52 variants ready within the window. Manual startup:
52/52 ready; median 7.45 s, maximum 10.71 s.
13 workbooks had at least one automatic-opening stall resolved under the tested manual mode.
Probes sometimes overlapped; these numbers are diagnostic observations, not a
controlled throughput benchmark. No password/repair/read-only dialog was observed
in these original cases. The startup blockers were not established to be modal prompts.

The 47-sheet workbook `native_000%2F3.295893.IJU2ZQGILXMCNPHHGR0LDNNFUXZL0WBXB.2.xls`
repeatedly remained unready after 45 seconds. A private copy with manual calculation
became ready in 8.74 seconds. Removing its 47 old printer-settings parts did not
resolve the stall. Formula and cached-cell XML were retained in the manual test.

## Changes

Before Excel starts, targeted private copies now request manual calculation and
disable fullCalcOnLoad, forceFullCalc and calcOnSave. Formula caches, input XLSX
and prepared bundles are unchanged. The explicit SD full rebuild still runs.
This changes the opening baseline: capture policy v5 rejects v4 comparison reuse,
and the Excel control-artifact cache has a new identity. Existing fresh Shrink
outputs remain valid because this patch does not change the shrinker.

Visible native password, repair, edit-lock, read-only, external-link, format,
circular-reference, security and memory dialogs are now checked before add-in
readiness and during pending commands. Unknown modal confirmations require an
owned disabled owner and at least two visible decision buttons. Progress dialogs
and ordinary security bars do not trigger this rule. No dialog is accepted.
`blocking-dialog.json` retains reason, phase, PID/birth, window class and text;
unclassified dialogs retain their own observation. Inspections address the
private desktop explicitly and use bounded native text reads instead of potentially
blocking UI Automation providers. Editable inputs, including passwords, are not read.
Only retained owned process handles are terminated; resident cleanup gets a chance
to restore its temporary add-in registration. Successful captures still reset the
existing idle deadline. No larger timeout or permanent filename blacklist was added.

## Full pipeline and remaining limitation

The 47-sheet case completed a real v5 comparison with 32 captures each for O, SD
and rebuilt SD. A previously transient small case also completed. Both completed
again with two concurrent comparison groups on the final build (96 and five captures).

`native_000%2F3.839048.CF1JLW0PYO0BHZZ2FYJTX0SU2M5J1SQVB.1.xls` still failed to
answer the explicit rebuild within the diagnostic 45-second command budget.
It contains 3,606 `_xll.EURO` calls. No modal blocker was observed; missing add-in
or another calculation cause is a hypothesis, not an established diagnosis.
Cleanup completed and the next test workbook ran successfully. Production retains
its 120-second capture-idle fallback for such unexplained stalls.

## Verification and local evidence

85 focused targeted Rust tests and 18 report-contract tests passed. ZIP preparation
and selection tests verify preserved formulas/caches, Strict OOXML, child ordering
and idempotence. Native private-desktop tests verify early password/repair and
unknown-confirmation rejection, foreign-PID exclusion and owned-process exit.
Original case probes do not establish those dialog categories; their handling is
covered by synthetic real-window tests.

Raw observations, timestamps, opening results, full reports, probe input copies
and cleanup receipts are local under `.tmp/performance-audit/opening-blockers`
in the main workspace. The initial force-terminated observation harness left
116 own temporary add-in registrations; these were removed only after matching
each GUID to its exact generated manifest and experiment-owned source path.
The harness and production early-skip path now request resident cleanup first.

Per-workbook results and machine/build identity are in
[`excel-opening-stalls-20261104.json`](excel-opening-stalls-20261104.json).
