# Enron overnight run: no completed visual comparisons, 2–3 November 2026

## Outcome and scope

The selected larger half of Enron finished preparation, but produced **no
completed visual comparisons and no saved image comparisons** before the user
restarted an almost unresponsive Windows machine. Preparation artifacts remain
usable. Missing visual evidence must not be interpreted as visual equality.

This is a historical incident report, not a validated fix or a finding that
Excel Shrink damaged workbook content. It records the state investigated on
3 November 2026, before any subsequent fixes on the experiment branch.

- Machine: `THE_LIBRARY`; Windows, desktop Excel.
- Source revision: `b1999d670898ffd08346cbc7e2a20934b822b0f3`, branch
  `experiment/officejs-oversized-capture`.
- The executable also contained an uncommitted, local conversion-reuse helper.
  It checked source identity and the source/converted SHA-256 hashes, restored
  only the pre-shrink XLSX, and generated fresh Default shrink outputs.
- Executable SHA-256:
  `dfdeda9e9426af063b7fa13cfa76c386e285e8fda1296679a6253a7f6fb01421`.
- Original corpus: 20,872 files; size partition `2/2`, 10,436 larger files,
  processed in descending original-file size.
- Workflow: `--targeted-comparison`, Original/Default with Check Performance.
- Controls: 24 preparation processes, two shrink threads per process, four
  comparison groups, 600-second open timeout, 1,800-second job timeout,
  `system_reserve_mib=0`. The saved legacy sweep control was one process;
  the comparison group target was four.
- Local times below are **Europe/Berlin, CET (UTC+01:00)**. The run started on
  2 November at 21:32:12; its final progress snapshot is from 3 November at
  08:49:58. Windows booted again at 08:50:23.

## Persisted progress

| Outcome | Workbooks |
| --- | ---: |
| Selected | 10,436 |
| Successfully prepared | 10,110 |
| Preparation unsuccessful | 326 |
| Targeted comparisons attempted | 483 |
| Comparison failures persisted | 480 |
| Still marked `comparing` at reboot | 3 |
| Prepared, not yet processed | 9,627 |
| Successfully completed visual comparisons | **0** |

All 480 available partial/final comparison reports also contained zero image
comparisons. These counts describe persisted evidence; unsaved in-memory work
cannot be recovered from the post-reboot investigation.

The last `progress.json` counted 479 failed comparisons and four in progress.
One additional failure had reached its workbook record and `failure.json`, but
had not yet reached that progress snapshot. The record inventory contained
9,627 `prepared`, 806 `skipped` (326 preparation + 480 comparison failures), and
three `comparing` records. The [aggregate evidence](enron-night-run-20261103.json)
preserves the reconciled counts.

## Comparison failures

| Persisted error | Cases |
| --- | ---: |
| `Unsupported or invalid worksheet row index` | 258 |
| `Excel command "optimize" did not reply within 600 seconds` | 203 |
| `Excel command "open_batch" did not reply within 600 seconds` | 15 |
| `Excel command "open" did not reply within 600 seconds` | 2 |
| UI Automation `GetNextSibling`, HRESULT `0x8000FFFF` | 2 |

### Confirmed parser defect in one affected workbook

Case `2a558c592b57cccae41e`, comparison directory
`targeted-9283135c-52f5-442e-9f27-f9149a923167`, failed before opening Excel.
Its `xl/worksheets/sheet1.xml` contains drawing anchors:

```xml
<xdr:row>5</xdr:row>
<xdr:row>7</xdr:row>
```

The [parser in the investigated revision](https://github.com/Alexander-Aue-Johr/excel-shrink-rust/blob/b1999d670898ffd08346cbc7e2a20934b822b0f3/excel-shrink-cli/src/regression/targeted_comparison/xml_changes.rs#L483-L490)
handles the local tag name `row` without restricting this branch to worksheet
rows in the appropriate namespace and parent context. It expects an `r`
attribute, substitutes `0` when it is absent, and rejects the result. A drawing
anchor's `xdr:row` is not a worksheet `<row r="…">`.

The inspected input's SHA-256 was
`044012e6df1cabb33e736b3b5f8c6b3ba1423c13fa5d2aac5b39c2e16f709a40`.
This explains this case without implying malformed cell rows or shrink-induced
corruption. The other 257 row-index failures were not individually inspected
for the same cause. Drawing anchors are described in Microsoft's
[DrawingML primer](https://download.microsoft.com/download/e/1/4/e14fb96f-83b8-4a2a-84db-7fa8acbe061a/Office%20Open%20XML%20Part%203%20-%20Primer.pdf).

## Windows instability and memory evidence

Application events in the run window, bounded by the reboot, recorded:

- **272 crashes of `WorkloadsSessionHost.exe`**, from 03:34:46 to 08:48:07.
  The faulting module was `onnxruntime_vitisai_ep.dll` from the
  `WindowsWorkload.EP.AMD.VitisAI.Framework` package, exception `0xc0000409`.
- **Five Excel crashes**, faulting module `aitrx.dll`, exception `0xc0000005`,
  at 03:48:16, 05:55:17, 07:15:01, 08:06:51 and 08:40:30.

No current low-memory diagnosis was found in System event 2004 or either enabled
Resource-Exhaustion Detector/Resolver Operational log. Those Operational logs
contained only service start/stop events in the inspected window. Absence of
such an event does not establish that memory pressure was absent.

The final four active sessions' launcher snapshots, sampled around
08:49:56–08:49:59, recorded these minima during their respective lifetimes:

| Measurement | Range across the four snapshots |
| --- | ---: |
| System available physical memory | 3.84–6.46 GiB |
| System commit headroom | **1.35–3.86 GiB** |
| Last current private job bytes, per sampled session | 0.44–0.50 GiB |

`minimum_available_bytes` and `minimum_commit_bytes` are observed minima, not
configured thresholds; see the [launcher implementation at that revision](https://github.com/Alexander-Aue-Johr/excel-shrink-rust/blob/b1999d670898ffd08346cbc7e2a20934b822b0f3/excel-shrink-cli/office-capture/resident.ps1#L424-L425).
The zero system reserve provided no reserve-based stop. The snapshots do not
account for the historical memory of every process or driver and do not identify
a single leak. Post-reboot free disk space was approximately 461 GiB on C and
410 GiB on E; these values do not reconstruct disk usage throughout the night.

Windows Error Reporting also emitted reports referring to older GPU/Watchdog
dumps named 15 August 2025. Their reporting during this window is not sufficient
evidence of a new graphics-driver failure that night.

## Assessment and follow-up

The evidence establishes a parser defect, repeated Check Performance timeouts,
a Windows/AMD runtime crash loop, five Excel crashes, and periods of limited
commit headroom. It does **not** establish which component caused the desktop
unresponsiveness, whether Check Performance caused the runtime crashes, or a
permanent WebView memory leak. A shared cause remains possible.

Before another unattended run:

1. Correct namespace/context handling and test the inspected drawing-anchor case.
2. Isolate Check Performance in a small controlled run, comparing Windows/Excel
   crash events with a run that omits it.
3. Record system commit/physical memory and process counts over time, including
   processes outside the owned Excel/WebView jobs; revisit a nonzero reserve.
4. Require a successful end-to-end visual result before scaling back to a night
   run. Keep preparation success separate from visual validation success.

No mitigation was tested in this investigation, no runner settings were changed,
and the run was not restarted.

## Evidence provenance

Raw workbook data, screenshots, full event messages and machine-specific paths
are not committed. The portable aggregate JSON accompanies this report. On the
investigated machine, the raw evidence remains under these repository-relative
paths:

- `.tmp/regression/enron-upper-20261102/records/`
- `.tmp/regression/enron-upper-20261102/control-room/{progress,control}.json`
- `.tmp/regression/enron-upper-20261102/evidence/<case>/targeted-<run>/`
- `.tmp/performance-audit/night-20261103/results-summary.json`
- `.tmp/performance-audit/night-20261103/windows-application-events.json`

The aggregate JSON records raw-file SHA-256 values to distinguish the investigated
snapshot from later edits. Reproduce the counts by grouping workbook statuses and
`failure.json.error`, then counting `comparisons` and
`image_comparison_statistics.comparisons` in available reports. Preparation and
comparison failures must be counted separately. Bound Windows queries to the run
start and reboot; report-emission time alone does not date an older crash dump.
