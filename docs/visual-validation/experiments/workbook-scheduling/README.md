# Workbook and sheet scheduling experiment

`benchmark.py` exercises the ordinary Default CLI on four deterministic workloads.

- Sixteen small single-sheet workbooks.
- One workbook containing sixteen small sheets.
- Two workbooks containing two larger sheets each.
- Eight small workbooks mixed with two larger two-sheet workbooks.

Each row contains one numeric value and seven empty cells, subject to Default's
representative-preservation rules. These
fixtures measure scheduling on this specific sparse cleanup workload. Dense
values, inherited styles and very large chunked sheets need separate workloads
before making broader scheduling claims.

The matrix uses one thread with one active workbook, two threads with one active
workbook, and two threads with four active workbooks. Repetitions alternate the
configuration order. Every output filename and every decompressed output member
must match the first configuration. Inputs and binary identity are retained.
Each trial has a 120-second subprocess deadline and its own stdout, stderr and
Perfetto trace. No Excel process is required.

Use an immutable release executable and an empty output destination on a working
drive. Set `TEMP`, `TMP` and `EXCEL_SHRINK_LOG_DIR` to that drive. For example,

```powershell
python benchmark.py --binary G:\experiment\excel-shrink-cli.exe `
  --output G:\experiment\scheduling --large-rows 65536 --repetitions 3 `
  --workload-note 'Record concurrent work and machine configuration here'
```

The 2026-11-08 pilot used 4,096 rows in the larger sheets and completed all 24
trials with identical outputs. Most individual observations were below half a
second and varied substantially alongside Enron. This establishes a working
experiment entry point and bounded output equivalence. It does not establish a
preferred scheduler configuration. Receipts are retained at
`G:\deltasieve-work\scheduling-pilot-20261108`.

The expanded run used 65,536 rows and three repetitions, completing 36 trials
with identical output members. Median seconds for configurations `(1,1)`,
`(2,1)` and `(2,4)`, respectively, were approximately

| Workload | One thread, one workbook | Two threads, one workbook | Two threads, four workbooks |
| --- | ---: | ---: | ---: |
| Small workbooks | 0.386 | 0.193 | 0.133 |
| Many sheets | 0.216 | 0.263 | 0.364 |
| Larger sheets | 0.627 | 0.508 | 0.811 |
| Mixed | 0.766 | 0.748 | 0.475 |

Enron was active throughout. These subsecond observations remain confounded by
other work and startup variation. They do not establish a general concurrency
benefit or justify a scheduling change. Receipts are retained at
`G:\deltasieve-work\scheduling-expanded-20261108`.
An independent two-sheet check confirmed every expected numeric coordinate and
value. Fixture generation produced byte-identical ZIPs on repetition. Default
retained one empty representative per row in that check, which the workload
description deliberately permits.

Peak process memory and CPU time are not measured by this harness. The measured
wall interval includes executable startup and output writing. Trace generation
also has a cost and is enabled for every configuration. Controlled throughput
claims require an isolated workload, larger repeated trials and additional
resource measurements. No production scheduling change follows from this pilot.
