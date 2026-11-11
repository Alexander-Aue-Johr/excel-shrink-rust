# Aggregate replay recording cost

## Scope

The ignored Rust test `corpus_sized_recording_retains_bounded_chunks` writes
100,000 synthetic complete progress checkpoints, representing five updates for
each of 20,000 workbooks. Timestamps advance by one second independently of
execution time. No Excel operations or scientific workbook comparisons occur.
The existing Enron runner remained active during this debug-build measurement.

The test reads every compressed chunk, checks its SHA-256, decodes it and verifies
all event sequences and timestamps. It checks the existing per-chunk byte and
event limits. The output directory must be new, with an existing parent.

```powershell
$env:REPLAY_COST_OUTPUT = 'G:\deltasieve-work\replay-cost-100k-20261108'
cargo test -p excel-shrink-cli corpus_sized_recording_retains_bounded_chunks `
  -j 2 -- --ignored --nocapture
```

Set TEMP, TMP and the project's log/build directories to a working drive before
running the command. Choose a different output path for subsequent trials.

## Observed result

| Measurement | Observed value |
| --- | --- |
| Checkpoints verified | 100,000 |
| Independent gzip chunks | 3,125 |
| Recording wall time, including terminal manifest publication | 17.369 seconds |
| Compressed chunk bytes | 1,777,877 |
| Manifest bytes | 865,134 |
| Append-only journal bytes | 658,326 |
| Largest pending serialized event payload | 6,386 bytes |
| Largest decoded chunk including array delimiters | 6,625 bytes |

The retained receipt is `G:\deltasieve-work\replay-cost-100k-20261108\cost.json`.
The combined history files occupy 3,301,337 logical bytes, excluding filesystem
allocation overhead and the measurement receipt. Many small chunks can consume
more allocated disk space than their logical sizes suggest.

## Interpretation and remaining checks

### Fresh-process memory measurement on 2026-11-11

The same ignored test was repeated in a fresh existing debug test executable,
with exactly one selected test and a 90-second timeout. Production captures
remained active. The existing open/save benchmark helper queried Windows
`GetProcessMemoryInfo` through the retained process handle after termination.
Its process-lifetime peak counters include recording and verification.

| Measurement | Observed value |
| --- | --- |
| Test result | 1 passed, 0 failed |
| Complete process wall time | 19.464 seconds |
| Recording wall time | 17.938 seconds |
| Peak working set | 15,597,568 bytes (14.88 MiB) |
| Peak committed process memory | 10,285,056 bytes (9.81 MiB) |
| Verified checkpoints and chunks | 100,000 and 3,125 |

Logical history sizes and serialized bounds matched the earlier trial exactly.
The receipt `replay-cost-20261111.json` records the executable SHA-256, scope and
measurements. This is one debug-process trial under concurrent load. It does not
isolate incremental recorder memory from the test harness or measure the browser
and additional detailed recording channels.

This single trial supports the practicality of compact aggregate history at this
checkpoint count. It measures wall time under concurrent load, without a matched
recording-disabled run. It establishes neither a throughput improvement nor a
production overhead percentage. Serialized byte counts do not measure allocator
overhead or the growing in-memory chunk metadata index. The subsequent
fresh-process measurement above supplies a whole-process peak at this scale.

The complete manifest fits within the current 8-MiB reader bound at this scale.
Much longer or more detailed recordings still require index-growth evaluation.
Browser seeking, bounded cache behavior and controls have separate tests and a
128-checkpoint browser acceptance. Process, sheet, capture and heartbeat history
remain additional recording channels to implement and measure.
