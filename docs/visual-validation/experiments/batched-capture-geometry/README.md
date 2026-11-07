# Batched capture geometry

This experiment measures the unchanged production `measure` operation against
`measure_batch` in one isolated hidden Excel instance. It creates a small XLSX
with uniform rows, one custom-height row and a hidden block. Sixty-four ranges
cover the sheet. Opening is excluded from timing. A warm-up batch precedes the
scalar/batch/batch/scalar sequence. Every returned geometry must match exactly.

```powershell
python docs/visual-validation/experiments/batched-capture-geometry/run.py `
  --binary target/release/excel-shrink-cli.exe `
  --output V:/excel-shrink-paper/external/excel-shrink-rust/.tmp/geometry-new-test
```

Use a new output directory. The script closes its owned resident in `finally`.
It does not capture images, modify corpus evidence or start a corpus run.

## Observation on 2026-11-07

| Mode | Ranges | First wall time | Second wall time |
| --- | ---: | ---: | ---: |
| Scalar | 64 | 16.343 s | 16.250 s |
| Batched | 64 | 0.250 s | 0.250 s |

All measured geometries matched. The hidden block remained represented by
zero-height ranges. Times include resident command publication and observation,
including the probe's 250 ms response polling interval. They are not isolated
Office.js timings or a corpus-wide throughput prediction. Rust tests separately
check partition equality, exact coverage, bounded batches and invalid receipts.
