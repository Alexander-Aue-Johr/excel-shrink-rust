# Batched Office.js geometry and image captures

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

Add `--captures` to compare image packages with sizes 1, 2, 3, 4 and 8 in the
same open workbook. It compares decoded RGBA pixels and dimensions for every
visible image, plus explicit zero-area observations. Only the package size varies.

Use a new output directory. The script closes its owned resident in `finally`.
It modifies no corpus evidence and starts no corpus run. The optional capture
experiment uploads PNG bytes through the production authenticated indexed route.

## Observation on 2026-11-07

| Mode | Ranges | First wall time | Second wall time |
| --- | ---: | ---: | ---: |
| Scalar | 64 | 16.343 s | 16.250 s |
| Batched | 64 | 0.250 s | 0.250 s |

The later combined run reproduced scalar times of 16.094 s and batched times
of 0.250 s. The machine was also completing corpus comparisons during the test.

## Image packages

The same twelve regions include eight visible regions and four fully hidden
regions. A package batches geometry reads into one synchronization and image
exports into another. Zero-area regions export no invented images. Binary uploads
then retain the command and image-slot identity for each result.

| Requested regions per package | Wall time for all twelve regions |
| ---: | ---: |
| 1 | 3.125 s |
| 2 | 1.579 s |
| 3 | 1.093 s |
| 4 | 0.828 s |
| 8 | 0.579 s |

Every image's decoded dimensions and RGBA SHA-256 matched the size-one baseline.
The four zero-area results also matched. These are sequential measurements in
one warmed-up worker, not randomized or independent samples. Larger corpus
images can dominate rendering and encoding time, and the shared 32-megapixel
package budget can reduce the actual number of images below the requested count.

The checked-in `results.json` retains timing samples, binary identity and confirmed
Excel exit. Full command responses, uploaded images and process metrics remain
in the selected output directory. Rust and JavaScript tests additionally cover
missing/reordered replies, invalid limits, zero-area geometry, over-budget
packages, indexed upload ownership and partial upload failure.

All measured geometries matched. The hidden block remained represented by
zero-height ranges. Times include resident command publication and observation,
including the probe's 250 ms response polling interval. They are not isolated
Office.js timings or a corpus-wide throughput prediction. Rust tests separately
check partition equality, exact coverage, bounded batches and invalid receipts.
