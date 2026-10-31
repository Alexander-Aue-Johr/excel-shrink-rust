# Redundant white row fills in the Enron corpus

On 2026-10-31, two prepared Enron workbooks exposed a tail-pruning gap on
worksheet `Feb`. All columns A:XFD already had the same solid white fill.
Empty rows repeated that fill and retained font/alignment settings below the
last value. Maximum removed their cells but retained the row records; Default
also retained empty font overrides as tail anchors.

| Preparation ID | Original attachment | Last value | Redundant tail |
| --- | --- | --- | --- |
| `ab7e13baa69a29ef50f4` | `native_000%2F3.1167415.GBNHITXDIFLTFXQ1QU5BFYSV452NK4U1B.1.xls` | K486 | 487:49633 |
| `90e5499ce2287c6a9014` | `native_000%2F3.1167409.J5S5XW3WGA53AK01AAS5JSTDJCKRIDBTB.1.xls` | E485 | 486:49633 |

Prepared original XLSX SHA-256 values, in the same order:

```text
ff9d44ffb2cba4d905bdb928e766730861fbb0051ef154ec20e2d7cc565a25bf
17ed6a278e2c1fe5c7220aa64dae50413e1d19aca9fb50925e673b8361065beb
```

## Rule and cost

Column finalization checks interval boundaries, exact overrides and uncovered
columns once, and caches an optional full-width fill ID. Row-tail checks may
ignore a fill only when its ID matches that cache. Borders remain anchors.
Fixed heights, grouping and unfamiliar row attributes remain conservative.
There is no additional worksheet pass or allocation per row; newly eligible
row headers receive a bounded attribute check.

A formatted column alone no longer makes an empty cell a tail anchor. Effective
cell overrides still do. Font representatives remain locally preserved when
subsequent content requires their row's layout; they do not by themselves keep
an otherwise redundant trailing region. These rules apply to Default and Maximum.

The reduced fixtures and counterexamples are documented in
`excel-shrink-core/src/sheet/tail_fill_regression_tests.rs`. They refine the June
2026 font-override test by distinguishing subsequent content from an empty tail.

## Validation

| Input | Original XLSX bytes | New Default bytes | New Maximum bytes |
| --- | ---: | ---: | ---: |
| `ab7e13baa69a29ef50f4` | 1,946,026 | 52,101 | 49,622 |
| `90e5499ce2287c6a9014` | 1,926,445 | 45,319 | 40,504 |

Both prepared originals and separate Excel-resaved controls were processed.
The white tails end at row 486 and 485 after cleanup. Values/formulas, including
their cached values, match the corresponding inputs; sources remain unchanged.

As a counterexample, copies had the empty tails colored solid blue (`#0070C0`).
Check Performance erased the blue tails. The updated Default and Maximum retain
them through row 49633. Excel COM CopyPicture at 100% zoom compared A475:L500
and A1000:L1015 against each resaved white/blue control: all 16 output comparisons
matched exactly. This is a targeted range check, not full-workbook visual proof.

The core tests passed (236 passed, one ignored), as did the CLI unit tests
(540 passed, 12 ignored, two excluded). The full suite also exposed an unavailable
corpus download, the existing prefixed-SpreadsheetML failure, and a profiling
integration failure (`missing writer.buffered_output`). The latter two were
reproduced with the unchanged experiment-branch core, independently of this patch.
