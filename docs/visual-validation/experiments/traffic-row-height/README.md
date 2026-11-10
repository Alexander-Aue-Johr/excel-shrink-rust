# Dense row promotion and inherited height

The Destatis workbook `verkehrsunfaelle-zeitreihen-xlsx-5462403.xlsx`
(SHA-256 `b320dc4f614eec1a1251efc5ca5041948761283ef33e29a5939652eab1fee806`)
exposes a row-height regression in sheet `3.8_(1)`, row 105
(`xl/worksheets/sheet51.xml`).

The original row has no `ht`, row style or `customHeight` attribute. Its sheet
default height is 12.75 points. Dense empty-cell formatting previously qualified
for promotion to a row style even without a fixed height. Excel then assigned
the promoted row a height of 9 points, moving the footer upward by 3.75 points
(five pixels in the native capture).

Row-style promotion now requires both an explicit `ht` and `customHeight="1"`.
The guard uses the existing parsed row flags before the promotion scan. Automatic
rows continue through ordinary cell cleanup and font-mask preservation. This
conservative restriction may retain more formatted cells. Fixed-height dense
and sparse promotion remain available.

## Reproduction

Use the original workbook and a new output directory. The environment variable
`TRAFFIC_WORKBOOK` below identifies the local original. The reference comparison
uses installed Microsoft Excel and the existing DeltaSieve native renderer.

```powershell
cargo test -p excel-shrink-core --lib
cargo build --release -p sheetshrink -p excel-shrink-cli --bin sheetshrink --bin deltasieve
$original = $env:TRAFFIC_WORKBOOK
$study = Join-Path $env:TEMP 'traffic-row-height-review'
$candidate = Join-Path $study 'cleaned/verkehrsunfaelle-zeitreihen-xlsx-5462403.xlsx'
& ./target/release/sheetshrink.exe $original (Join-Path $study 'cleaned') `
    --rayon-threads 1 --writer-workers 1 --max-active-workbooks 1
& ./tools/run-reference-comparison.ps1 `
    -Binary ./target/release/deltasieve.exe -Original $original -Default $candidate `
    -Plan ./docs/visual-validation/experiments/traffic-row-height/reference-plan.json `
    -Output (Join-Path $study 'reference') `
    -OperationTimeoutSeconds 180 -TotalTimeoutSeconds 600
```

The supplied plan covers `A1:U110`, including the table and footer. A successful
fix preserves the native row height, geometry and pixels in this area. The
reference command rebuilds and saves both workbooks before capture. Its receipt
and images establish the result only for the supplied area.

The core regression test exercises dense full-width and one-gap rows with
missing height, automatic height, incomplete custom height and disabled custom
height. It checks that cleanup never adds a row style and remains idempotent.
Existing tests retain coverage of fixed-height promotion.

## Native verification on 2026-11-10

The fix was built on `fd428d3` from `codex/datagov-validation-watch`. Both
comparisons used the same retained DeltaSieve producer from `a7eef1a` and Excel
16.0.20527.20150 x64. The original PNG SHA-256 was identical across both runs.
The corrected capture was also inspected visually.

| Observation | Previous cleanup | Corrected cleanup |
| --- | --- | --- |
| Native row 105 height | 9 points | 12.75 points, equal to original |
| Original image | 1358 × 1327 | 1358 × 1327 |
| Candidate image | 1358 × 1322 | 1358 × 1327 |
| Changed pixels in common image area | 3,408 | 0 |
| Geometry differs | Yes | No |
| XLSX size | 2,608,628 bytes | 2,654,536 bytes |

The corrected original/candidate PNGs are byte-identical. The output still
reduces the 37,207,883-byte original by about 92.9%. Its ZIP CRC check passed and
the input SHA-256 remained unchanged. The fresh single-thread cleanup took
13.28 seconds with statistics enabled. This single measurement is not a timing
study of the guard's overhead. Its extra decision uses only two parsed flags
and skips the promotion look-ahead for automatic rows.

The core suite passed 296 tests with one manual timing test ignored.
[`evidence.json`](evidence.json) retains binary, workbook and image hashes,
native geometry, row heights and the measured cleanup receipt. Full native
receipts and images are retained locally under
`C:/SheetShrinkStudies/row-height-fix-20261110/{before,after}`.

## Remaining whole-workbook planning investigation

A separate policy-v15 diagnostic on 10 November 2026 retained the rebuilt
states but reached no pixel verdict within its ten-minute wall-clock limit.
The process spent the final long phase in Rust before publication of the
`before_geometry_partition` index, with approximately 17 GiB private memory.
The operation was stopped after 600.48 seconds. Its provisional XML candidate
counts must not be reported as completed captures. An earlier diagnostic was
stopped prematurely after treating a provisional count as a final plan. Both
attempts remain separate incomplete observations.

The existing ignored `inspect_retained_background_extents` diagnostic then read
the retained `OR-rebuilt.xlsx` without Excel in 79.93 seconds. Of 215 sheets,
101 had archive-only visual bounds `A1:XFD1048576`, while 67 qualified for
background-tail sampling. For example, sheet `1.3` has cell-content bounds
`A1:AA95` but whole-grid visual bounds. Its inherited row and column styles have
solid fills and border ID zero. It also has drawing-related conservatism.
This archive-only diagnostic does not perform native drawing qualification,
so these counts cannot establish which bounds survive that qualification in
the real comparison.

The next investigation must reconcile the retained native drawing inventories
with the requested rectangles before grid expansion. `ContentBounds::select`
enumerates grid tiles before `capture_layout::coalesce` and before the parent
index is published. A large unresolved rectangle can therefore consume memory
without producing a capture-plan index. This identifies the diagnostic boundary,
but does not yet prove a safe reduction of this workbook's capture coverage.
No full-workbook validation success follows from the earlier single-area
row-height acceptance.
