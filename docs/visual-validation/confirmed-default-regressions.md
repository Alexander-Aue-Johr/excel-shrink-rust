# Confirmed Default visual regressions

Two visual-preservation defects in existing Enron **Default** outputs were
confirmed on 2026-10-29 by restoring only deleted empty formatting cells in
private copies. Both differences remained with Excel's green warning indicators
disabled. This document records the evidence; it does not claim that the
shrinker defects have been fixed.

The inputs were prepared by `fa19e0b` (executable SHA-256
`707ef5e197af88bcb552fcfc73f457e20ad911bd4ac1548f0db7b1e01d8c38db`),
using Default configuration hash `aac065865383a9a7`. Their original recorded
stored and recalculated cell-value difference counts were both zero, as were
the reported row-height changes.

## Cases

Both archive members have the prefix
`enron_xls-5b73fc395cbe4727a986ab02a5028c1c1585617f/edrm/`.

| Record ID | Source member suffix | Worksheet |
| --- | --- | --- |
| `08096660a53769fc88ad` | `native_000%2F3.655709.IHPVV2IBFDPT0FPCUWU0WDKLZ4TNF2WHB.1.xls` | `Position`, index 1 |
| `13c8342137c83f328686` | `native_000%2F3.815224.ACVYIOT13NBCSRZOQDFUK2Z0WEBIQ1RDA.1.xls` | `Raw Data`, index 1 |

**Position: center-across-selection span.** Default visibly shifts `TOTAL ECT`,
`TOTAL HANNON` and `Total ECT 12 Month`. Their populated cells `E66`, `I66` and
`M66` retain style 53, with `horizontal="centerContinuous"`. Original empty
cells `G66,J66,K66,N66,O66,P66,Q66`, also style 53, were deleted. Restoring only
these seven exact original `<c r="..." s="53"/>` fragments into Default made
the `A62:U71` capture pixel-identical to Original. The unmodified Default capture
remained different. The tested image size was 1098 × 171 pixels.

**Raw Data: gray cell fill.** Empty cells `F187:F190` with explicit style 17
were deleted, and their gray fill becomes white in Default. Restoring only
these four exact original `<c r="..." s="17"/>` fragments made the `A186:H190`
capture pixel-identical to Original; unmodified Default remained different.
The tested image size was 370 × 86 pixels. The XML suggests an incorrect
row/column-style inheritance decision: column F has a gray style, while the
affected rows have `customFormat="1"` with implicit style 0. The restoration
proves the visible effect of deleting the cells; the precise code-level
inheritance error still needs a fix and a focused regression test.

## Controlled method and result

1. Extract `original.xlsx` and `default.xlsx` from each existing prepared ZIP.
2. Make a third copy of Default. Insert only the listed original empty cell
   fragments, in cell order, into `xl/worksheets/sheet1.xml`.
3. Verify that removing those inserted fragments reproduces the Default sheet
   XML exactly. Verify every other ZIP part's uncompressed SHA-256 is unchanged
   (47 other parts for Position; 22 for Raw Data). Do not change styles, values,
   formulas, row heights, row metadata, columns or the other worksheets.
4. Open Original, Default and the restoration control in separately owned Excel
   instances on a private capture desktop. Verify instance-scoped
   `ErrorCheckingOptions.BackgroundChecking=false` before releasing capture
   jobs. Restore the prior setting during cleanup.
5. Capture each stated region twice using Office.js. Compare decoded RGBA
   pixels, not PNG encoding bytes. Both repetitions of each input must agree.

Both cases passed: Original differed from Default, while Original and the
restoration control had identical RGBA SHA-256 values. Every same-input repeat
was stable. Source prepared ZIPs and extracted input XLSX hashes remained
unchanged. The native marker-suppression test separately verified that actual
`#REF!` and division-by-zero formula/error text was preserved.

This establishes causality for the two stated regions. It does not certify the
entire workbooks, prove a general rule about all style inheritance, establish
profile monotonicity, or explain the other Default differences in the study.
The control is a diagnostic copy, not an accepted replacement shrink result.

## Reproducing from existing preparations

Keep the original Enron ZIP and the study's prepared bundles. For each record
above, the existing bundle is `prepared/<record-id>.zip`; verify its source
identity before extracting `original.xlsx` and `default.xlsx`. Restore the
listed cell fragments and repeat the controlled method. Compare all three
inputs under the same capture configuration, with warning indicators disabled
and exact RGBA comparison. Do not infer repair from filenames or workbook status.

On the investigation machine, local helpers and receipts are available under:

- `.tmp/default-cell-controls/prepare-controls.ps1` and `EmptyCellControl.cs`;
- `.tmp/default-cell-controls/20261029-105259/receipt.json`: exact restored XML,
  input/control hashes, unchanged-part counts and regional capture plans;
- `.tmp/default-cell-controls/captures-20261029-105708/receipt.json`: before/after
  RGBA hashes, repeated-capture checks and twelve retained PNGs;
- `.tmp/default-cell-controls/confirmed-default-regressions.txt`: detailed notes.

**These are machine-local investigation artifacts, not files bundled in the
repository.** On that machine, after obtaining an exclusive Excel test slot,
the prepared controls can be recaptured with:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .tmp/default-cell-controls/capture-controls.ps1 -ControlsReceipt .tmp/default-cell-controls/20261029-105259/receipt.json -Run
```

A fresh checkout needs the matching corpus/preparations and equivalent control
construction; the command above is not a portable, repository-provided test.
