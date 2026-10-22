# Paired comparison observations — 22 October 2026

This is a bounded laptop measurement, not a corpus benchmark or a measurement of
Excel's internal scheduling. No production regression code was changed.

## Workload and method

- Windows 11 Enterprise, Ryzen 5 Pro 7535U, 12 logical processors, 32,065 MiB RAM.
  Other applications and WSL were resident. The admission budget used currently
  available RAM, not the machine's total installed RAM.
- `krankenhausverzeichnis-3500100227005.xlsx`, explicit selection
  ` KHV_2022!A1:FP400` (the sheet name begins with a space).
- Four capture tiles per side, 32 MP per tile, visible context, selective overflow.
- Main runtime `6a0ef2615f922665c4a4f7b4c24e58204f53d6ea`.
- Maximum-cleaning output prepared once. No visual differences in this selection.
- Three measured repetitions per configuration, seed 73021, randomized order;
  startup pilots excluded. All workers reproduced the same visual result.
- Each output folder retains frozen sources, inputs, manifest identities, raw
  observations and logs. The later launcher cleanup change only retries removal
  of temporarily locked workbooks; it does not change capture or comparison.

## Batch pilot

One pair, hash transport, aggregate budget 128 MP, three repetitions:

| Requested/effective batch | Median capture seconds | Peak job MiB |
| --- | ---: | ---: |
| 1 / 1 | 8.034 | 1,506 |
| 2 / 2 | 8.051 | 1,627 |
| 4 / 4 | 7.860 | 1,728 |

Batch 1 was selected by the predeclared rule: the smallest batch within 5% of
highest median throughput. Larger batches did not give a useful gain here.

## Parallelism matrix

Batch 1, aggregate budget 128 MP, three measured repetitions per completed
configuration. One pair means two distinct Excel processes.

| Transport | Pairs | Median capture s | Pairs/min | Median setup s | Median total s | Peak job MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Binary PNG | 1 | 15.990 | 3.752 | 24.783 | 40.739 | 1,525 |
| Binary PNG | 2 | 17.069 | 7.030 | 31.922 | 49.281 | 2,685 |
| PNG hash first | 1 | 7.905 | 7.590 | 24.754 | 33.557 | 1,472 |
| PNG hash first | 2 | 9.747 | 12.312 | 33.056 | 43.476 | 2,666 |

Relative to one pair, two pairs achieved 1.87 times the capture throughput for
binary PNG and 1.62 times for hash first. The hash mode uploaded zero PNG bytes
because every tile's digest matched. This workload favors that fast path;
it does not establish its advantage for worksheets with many visual changes.

There were 16 complete observations including four startup pilots. All 36
observations for 3–8 pairs were rejected before Excel launch by the conservative
RAM admission guard. There were no capture failures in this matrix. These rows
produce exit code 1 because the requested matrix was not fully executed. They
must not be plotted as zero throughput or interpreted as an Excel scaling limit.

One complete run had 71.8 seconds of startup but only 7.9 seconds of capture.
Do not use capture-only throughput to predict whole-corpus completion time.
A fresh Tower run should measure more workloads, including differing sheets,
and all worker counts that its memory budget safely permits.

## Correctness and local artifacts

The Windows protocol suite passed 50 assertions, covering PNG encoding-only
changes, changed pixels, incomplete coverage and eight independent pair states.
The Node batch-budget checks passed. Real A/A controls were equal; an A1 text
injection was detected by both transports. Its cropped images were inspected.
A final A/A run also verified that the cleanup retry removed its temporary stage
without a cleanup warning. Earlier development stages and stale registrations
were removed only after checking that no experiment process still used them.

Local results under the main checkout (not committed):

- `.tmp/concurrent-pairs/batch-pilot/report.html`
- `.tmp/concurrent-pairs/scaling-matrix/report.html`
- `.tmp/concurrent-pairs/control-equal/report.html`
- `.tmp/concurrent-pairs/control-change-verified/report.html`

Use the native Windows commands in README.md for a new run. Keep separate output
folders and do not pool results from different scopes or machines.
