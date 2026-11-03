# Controlled Enron restart: one and two groups, 3 November 2026

Follow-up to the [failed overnight run](enron-night-run-20261103.md) on the same
`THE_LIBRARY` machine. The historical report remains a record of the earlier
build; the results below are subsequent controlled tests, not revised night-run
counts. All clock times are Europe/Berlin. This work is on
`docs/enron-night-run-findings-20261103`; the shared experiment branch was not
modified.

## Changes applied

- Merged the other machine's long-Windows-path fixes `b462272` and `be0a8d2`.
- `31c08e5`: restrict worksheet rows, columns and default row format handling to
  their worksheet context. Drawing anchors no longer become worksheet rows.
- `9fc1821`: commit the existing, now retry-safe verified conversion-reuse helper.
  This is independent of approving complete old shrink preparations.
- `6ecb27c`: locate native task panes only within the exact owned Excel window,
  then traverse the pane's accessibility **control** view. Avoid searching the
  worksheet's entire raw accessibility tree. Recognize German pane/completion
  text as well as English, retaining positive completion confirmation.

The initial fixed-parser test still timed out in `optimize` after 90 seconds.
Tracing established that `ExecuteMso('CheckPerformance')` returned and execution
remained in pane discovery. Native pane scoping found `Arbeitsmappenleistung`
and sent the action, but an intermediate build still failed to acknowledge
completion. With control-view traversal and localized positive confirmations,
real Excel reported `Arbeitsmappe optimiert` on one input and
`Keine Optimierung erforderlich` on the other. These changes were tested as a
combined fix; their individual contributions to traversal latency are not
quantified.

## Matched one-group and two-group tests

The same inputs were run separately, then together in independent private
desktops. Each simultaneous test had its own **fresh Check Performance cache**,
so a cache hit could not hide native optimization work. Two groups means two
comparison jobs, not two Excel processes: each job can open the Original,
Default, Check Performance and their rebuilt states.

| Input / mode | Duration (s) | Captures | Image comparison operations | Check Performance |
| --- | ---: | ---: | ---: | --- |
| Small, one group | 47.351 | 18 | 69 | Optimization confirmed |
| Large, one group | 100.352 | 72 | 276 | No optimization needed, confirmed |
| Small, concurrent pair | 49.216 | 18 | 69 | Optimization confirmed |
| Large, concurrent pair | 99.734 | 72 | 276 | No optimization needed, confirmed |

Small is the previously failing drawing-anchor case `2a558c592b57cccae41e`
(three selected regions); large is `02aab9988d84bef6beb6` (12 selected regions),
which previously timed out in Check Performance. The larger test used its
`original.before-save.xlsx` / `default.before-save.xlsx` controls.

All four reports ended as `completed_with_limitations`. Their selected coverage
is bounded by the targeted comparison policy; completion does not certify an
entire workbook's visual equality. Capture counts include six retained states.
Image comparison operations can exceed the number of unique PNGs.

After normalizing only the output-directory prefixes, the one-/two-group
reports matched exactly for:

- findings, including geometry, difference flags and PNG references;
- staged image comparison results;
- selected content coverage;
- capture counts.

No two-group-only comparison failure or changed verdict was observed in this
sample. Excel processes were closed after the tests. The pair's wall time was
about 100 seconds versus about 148 seconds for the two sequential runs, but
these two inputs are not a corpus throughput benchmark or a long-run soak test.

## Resources and remaining Windows failures

A two-second sampler recorded system available memory, commit headroom, paging
and process counts. Sampling began partway through the larger standalone test.

| Sampled measurement | One group | Two groups |
| --- | ---: | ---: |
| Minimum available physical RAM (GiB) | 14.70 | 12.28 |
| Minimum available commit headroom (GiB) | 22.78 | 18.65 |
| Peak observed Excel processes | 6 | 9 |
| Peak observed Windows workload hosts | 10 | 12 |

The Windows/AMD runtime issue is **not fixed** by these changes.
`WorkloadsSessionHost.exe` crashes in `onnxruntime_vitisai_ep.dll` remained:
there was one in each standalone successful test, one in the small concurrent
window, and three in the large concurrent window. The concurrent windows
overlap, so these event counts must not be summed. No Excel crash occurred
inside the four successful test windows. The earlier no-Check-Performance
control also completed, with 12 captures and 36 image comparisons and no
Windows workload/Excel crash within its approximately 29-second interval.
This limited observation does not establish a causal link to Check Performance.

The new tests establish working comparisons and substantial memory headroom at
one and two groups, not that the prior desktop-freeze cause is fully resolved.

## Height-measurement failure found during corpus continuation

After the matched tests, the real corpus produced a new failure in case
`87c8ac003fd17e1f7abe`: `Targeted height measurement deadline exceeded; coverage
is incomplete`. Admission was paused and active groups drained before further
testing. The same input failed alone after 30.002 seconds, matching the failure
under two groups at 30.025 seconds. It was therefore not a two-group-only error.

The host silently capped the height budget at 30 seconds even when its overall
operation timeout was longer. `95c592b` forwards the configured operation budget;
the enclosing command deadline remains enforced. A protocol test with a mock
PowerShell host verifies that a 90-second setting reaches the receiver as
90,000 milliseconds.

The affected `NG_01` sheet requests **4,462 disjoint height spans**. The JavaScript
implementation awaited an independent measurement for each span, preventing the
existing 64-range Office batching from combining separate selections. Raising
the timeout alone allowed height checks to progress but retained this cost; that
diagnostic was stopped before a complete visual report.

`84996bb` batches all requested spans through the same bounded measurement queue.
It retains the original selections, absolute row numbers, gaps, adjacent-span
boundaries and exact coverage validation. Mixed and hidden heights still split
as required. Its regression test measures 200 disjoint spans in four Office
synchronizations rather than 200.

On the real input, confirmed responses measured all 4,462 spans in **2.746 seconds
for Original** and **2.968 seconds for Default**, returning `complete=true` and
4,462 runs in both cases. The separate diagnostic advanced into the large
visible-range planning phase without a new error. It was explicitly stopped
there to avoid duplicating the same ongoing work in the corpus; this is a
successful height-check verification, **not** a completed visual verdict for
that workbook. The corpus was resumed afterward with the new code and at most
two groups.

## Corpus continuation

The existing larger-half Enron run was resumed with `--compare-only`, initially
one comparison group. One real corpus workbook completed, with 84 captures
across its six states and confirmed Check Performance. Only then was the target
raised to **two**, including the saved launch configuration. Preparation/sweep
controls were also limited to two for a future restart. No test or resumed
corpus admission requested more than two comparison groups.

The source shrinker is unchanged from `b1999d6` through these fixes, so the
10,110 existing preparation bundles were explicitly approved by their producer
SHA-256. No re-conversion or re-shrinking was requested. Preparation failures
remain visible; `--compare-only` does not retry those conversions. Old failed
visual attempts are eligible for retry. Check Performance remains enabled;
the diagnostic request to omit it is superseded by the successful repair.

A 4,096 MiB system reserve replaces the earlier zero reserve to retain Windows
headroom. This uses the runner's existing memory guard; it is not a guarantee
against every driver or runtime failure. The operating configuration is saved
with two groups and the same `2/2`, descending-size corpus selection.

### Saved operating limits and resume command

The final launcher configuration and `control-room/control.json` both select
at most two comparison groups; preparation and sweep controls are two, and the
system reserve is 4,096 MiB. The local launcher is invoked with `-CompareOnly`.
The equivalent direct CLI form is shown below; request/output paths are local
run dependencies, not files shipped with this report:

```powershell
$env:EXCEL_SHRINK_SYSTEM_RESERVE_MIB = '4096'
.\target\release\excel-shrink-cli.exe regression study `
  --request-file .tmp/enron-full-hash-screening/runs/20261102-203209/request.json `
  --output-root .tmp/regression/enron-upper-20261102 `
  --download-root .tmp/archive-regression/downloads `
  --run-corpus Enron --size-part 2/2 --order size-desc `
  --machine-label THE_LIBRARY --resident-capture --targeted-comparison `
  --compare-only --comparison-workers 2 --control-room --control-room-port 8771 `
  --startup-timeout-seconds 600 --workbook-timeout-seconds 1800 --hide-excel `
  --reuse-prepared-from dfdeda9e9426af063b7fa13cfa76c386e285e8fda1296679a6253a7f6fb01421
```

The final observed corpus snapshot contained five completed workbook results
(four from the first continuation and one with the final height-batching build),
two active groups, and no new failed attempt in the final restarted run. At that
snapshot Windows had approximately 14.88 GiB physical RAM and 20.99 GiB commit
headroom available. These are a bounded handoff observation, not a promise that
all remaining files or the unresolved Windows runtime will be error-free.
Active observation ends after this controlled handoff; the corpus continues.

## Verification and evidence

- 79 targeted-comparison Rust tests passed after the budget fix; nine native/artifact tests ignored.
- 86 JavaScript geometry, targeted diagnosis and resident tests passed, including
  batched disjoint/adjacent selections and mixed/hidden height coverage.
- Verified conversion reuse, mismatched source/checksum rejection and replacing
  an incomplete prior output passed its focused test.
- Native hidden-window tests verify ownership/class filtering and reject a
  foreign process or missing owner.
- Pane tests cover the observed German success/no-work captions, English
  completion, and unknown/disabled states that must not imply success.
- Windows long-path tests and the final release build passed.

The [portable aggregate](enron-controlled-resume-20261103.json) records the
native test matrix, limitations and corpus run identity. Full native reports,
telemetry, error logs, semantic comparison and sampled CSV remain locally under
`.tmp/performance-audit/controlled-20261103/`; workbook contents and personal
paths are not committed. The historical machine specification is in the earlier
incident report.
