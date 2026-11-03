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

## Run identity, invocation and duration

| Identifier | Recorded value |
| --- | --- |
| Run ID | `c9d89584-85f3-4601-948f-f83c6b25dae9` |
| Machine label / hostname | `THE_LIBRARY` |
| Machine ID | `e0297e77534621d532d87c04518328e1d25f484db67239205d783751fa4b4ddc` |
| Output directory, relative to the repository | `.tmp/regression/enron-upper-20261102` |
| Launch receipt directory | `.tmp/enron-full-hash-screening/runs/20261102-203209` |
| Original runner PID | `31424` (historical; PID alone is not a stable identity) |
| Corpus ID | `219f5e4e42c4397a98d8e4e0cc2825e90ab3e622c4989ade93148363efb84bf5` |

The run manifest is `metadata/runs/c9d89584-85f3-4601-948f-f83c6b25dae9.json`
beneath that output directory. Its `machine_manifest` field identifies the exact
machine snapshot; that repository-relative path is retained in the aggregate JSON.
The source revision and executable hash above distinguish this locally built
executable from an unmodified build of the same commit.

The launch receipt's invocation, with absolute paths normalized to repository-
relative paths, was:

```powershell
.\target\release\excel-shrink-cli.exe regression study `
  --request-file .tmp/enron-full-hash-screening/runs/20261102-203209/request.json `
  --output-root .tmp/regression/enron-upper-20261102 `
  --download-root .tmp/archive-regression/downloads `
  --run-corpus Enron --machine-label THE_LIBRARY `
  --resident-capture --control-room --control-room-port 8771 `
  --comparison-workers 4 --order size-desc `
  --startup-timeout-seconds 600 --workbook-timeout-seconds 1800 `
  --size-part 2/2 --targeted-comparison --hide-excel
```

The local starter also set `EXCEL_SHRINK_CONVERTED_BUNDLES_DIR` to
`.tmp/regression/enron-full-20261030-backup-20261101-062355/prepared` and
`EXCEL_SHRINK_SYSTEM_RESERVE_MIB=0`. The request JSON and these converted inputs
are dependencies of that recorded invocation, not files included in this report.

| Timing | Duration |
| --- | ---: |
| Process launch → last persisted progress | 11 h 17 min 46.561 s |
| Runner-reported preparation active time | 1 h 23 min 42.162 s |
| Runner-reported comparison active time | 9 h 53 min 52.047 s |
| Runner-reported total active time | 11 h 17 min 34.209 s |
| Process launch → Windows boot after restart | approximately 11 h 18 min 11 s |

The process started at `2026-11-02T21:32:12.2674745+01:00`; the run manifest was
created at `21:32:25.083+01:00`. The last progress timestamp was
`2026-11-03T08:49:58.828+01:00`. Initialization and checkpoint timing explain why
wall time and runner active time differ. These intervals do not establish
continuous CPU use or the precise instant when Windows became unresponsive.

## Machine specification and collection time

The run's persisted machine manifest records the following historical snapshot:

| Component | Recorded during the run |
| --- | --- |
| Manufacturer / model | Micro Computer (HK) Tech Limited / AI Series |
| CPU | AMD Ryzen AI 9 HX 370 w/ Radeon 890M |
| CPU topology | 12 physical cores, 24 logical processors |
| Process architecture | x86-64 |
| OS | Microsoft Windows 11 Pro, 64-bit, version `10.0.26300`, build `26300` |
| RAM reported by the runner | 31,802,109,952 bytes, approximately 29.62 GiB OS-visible |
| Excel | Click-to-Run, x64, version `16.0.20527.20112` |
| Office product ID | `O365HomePremRetail` |

Additional read-only inspection on **3 November 2026, after the reboot** found:

| Component | Observed after reboot |
| --- | --- |
| Installed RAM | 32 GiB, two 16 GiB modules; WMI `ConfiguredClockSpeed=5600` |
| Graphics | AMD Radeon(TM) 890M Graphics |
| Graphics driver | `32.0.21030.2001` |
| Windows display version / full build | `26H2` / `26300.9711` |
| C drive physical disk | Samsung SSD 990 PRO 2TB, NVMe SSD, 2,000,398,934,016 bytes |
| E drive physical disk, repository/results | KINGSTON OM8PGP41024Q-A0, NVMe SSD, 1,024,209,543,168 bytes |

CPU, OS base build, Excel version and OS-visible RAM matched the historical
manifest. Installed RAM, module configuration, graphics/driver details, disk
models/mapping and Windows revision were not collected in that historical
manifest; their post-reboot observation does not independently prove they were
identical throughout the run. Installed RAM and OS-visible RAM are different
measurements and must not be used interchangeably as the available job budget.
No serial numbers, hardware UUIDs or personal account names are included.

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
