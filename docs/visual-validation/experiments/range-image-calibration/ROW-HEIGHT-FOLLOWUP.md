# Row-height follow-up: fonts, fixed heights and geometry timing

This is an additional diagnostic suite (`row-heights`), not a production fix. It does
not change screenshot bounds, trim PNGs, tile the hospital workbook or replace the
original calibration fixtures. `all`, `units` and `markers` keep their previous scope.

## Run and return results

Apply `patch_row_height_followup.py --dry-run`, then `--apply`, from the repository root.
Python 3.10+ and the standard library suffice. Fixture packages are constructed directly
from XML during the patch. No Excel launch occurs during either patch mode.

Run **Experiment: visual validation -> GUI -> Get picture units and markers** and select
**Row heights: fonts, customHeight and geometry timing**. Leave the manual/debug pauses
unchecked for the first measurement. Extended error logging stays enabled.

From a Windows PowerShell terminal at the repository root:

```powershell
& .\docs\visual-validation\experiments\range-image-calibration\run.ps1 -Suite row-heights -NonInteractive
```

From a WSL terminal at the repository root:

```bash
script="$(wslpath -w "$PWD/docs/visual-validation/experiments/range-image-calibration/run.ps1")"
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File "$script" -Suite row-heights -NonInteractive
```

Optional debugging: `-WaitForStart -BreakOnError -UseOfficeDebugJs`.

At the end, the terminal prints **REVIEW BUNDLE:** followed by a path such as:

```
docs/visual-validation/experiments/range-image-calibration/results/row-heights-review-<run-id>.zip
```

Return that ZIP, rather than only probes.csv. It includes the exact synthetic XLSX inputs,
manifest, result.json, row-height-summary.json, row-geometry.csv, probes.csv, original PNGs,
and archived host/progress logs. It contains no hospital input; logs do include local paths.
Even a failed run attempts to assemble a bundle, using available progress/logs. If bundle
creation fails, zip the **Run archive:** folder printed in the terminal instead.

Latest JSON/CSV are separate from the old calibration results:

```
results/officejs-dotnet-row-heights.json
results/officejs-dotnet-row-heights.csv
```

Per-run raw files are under `.tmp/visual-validation/range-image-calibration/runs/<run-id>/`.
A second independent run is useful only if the first reports instability or a read-order
contrast; keep both run-ID ZIPs in that case. `status=complete` means the measurements
completed, not that any particular minimum-height hypothesis was proved.

## Why these tests

The user observed that automatic rows can grow for larger fonts while explicitly fixed
row heights remain fixed and may clip glyphs. That is a more appropriate explanation
for font-dependent heights than assuming a geometry bug merely from the font contrast.
The previous fixture generator also set `sheetFormatPr@customHeight="1"` even on `Defaults`.
It omitted `ht`/`customHeight` on the individual rows there. These two levels must not be
conflated.

The remaining engineering questions are when the height is determined and whether raw
row geometry, the sum of individual rows, an aggregate Range.height and PNG pixels agree.
The previous 12.6-point fixed-row measurements leave a substantial sum-versus-raster
rounding question even without any automatic font-dependent height change.

## Three new deterministic inputs

### vv-row-height-calibri11.xlsx

The Normal font stays Calibri 11. A matrix varies the cell font separately (8, 11, 24,
48 pt), with numeric/text cells without an explicit style, styled empty cells, completely
absent cells/rows, row-inherited fonts, a cell overriding its row font, mixed fonts in one
row, and wrapped text. There are no labels in measured cells: labels and XML attributes
are in the manifest, so annotations cannot accidentally enlarge a row.

The same 19 cases occur on eight sheets:

| Sheet | defaultRowHeight | sheet customHeight | row ht | row customHeight |
|---|---:|---|---|---|
| Auto126 | 12.6 | absent | absent | absent |
| Auto300 | 30 | absent | absent | absent |
| SheetCustom126 | 12.6 | 1 | absent | absent |
| SheetCustom0 | 12.6 | 0 | absent | absent |
| HtNoFlag126 | 12.6 | absent | 12.6 | absent |
| RowCustom0 | 12.6 | absent | 12.6 | 0 |
| RowFixed126 | 12.6 | absent | 12.6 | 1 |
| RowFixed300 | 30 | absent | 30 | 1 |

An absent row necessarily becomes an explicit empty row in policies that assign it an
individual height; `xml_row_present` records this, rather than pretending those XML
representations are identical. No column font is changed across a matrix: a large font
on a whole column could affect every row and would confound the intended cell test.

Two separate runtime sheets compare setting a cell font from 11 to 48 to 8 points in an
automatic row versus a fixed row. Only the LAST step explicitly invokes AutoFit. Changes
are made only to disposable experiment copies, never source fixtures or corpus inputs.
Source XML metadata remains explicitly labelled as pre-edit metadata.

### vv-row-height-calibri16.xlsx

The three most diagnostic policies (Auto126, SheetCustom126, RowFixed126) repeat with
Normal Calibri 16, while the explicit cell-font variants remain exactly the same. This
separates Normal-font effects from a change of the whole test's explicit cell fonts.

### vv-row-height-geometry.xlsx

Normal font: Calibri 16. Four pairs of byte-identical worksheet XML are tested with
1000 rows each: default 12.6 with sheet customHeight=1, default 12.6 without that flag,
fixed individual 12.6, and fixed individual 12.75 (aligned 96-DPI pixel reference).
The initial active sheet is an unrelated Cover.

For each pair one sheet is queried first, the other is rendered first with **no diagnostic
geometry load before its initial getImage**. "Cold" does not claim Excel has never laid
out the sheet: opening the workbook can itself initialize layout.

Phases capture the aggregate geometry before individual reads, all 1000 individual row
heights/tops/RangeFormat.rowHeight values, and the aggregate again afterward. The phases
cover initial render, a repeat after row reads, explicit sheet activation, disjoint part
images, individual samples, and a final full repeat. Activation and AutoFit are never
silently performed as preconditions of these causal measurements.

Disjoint parts cover 1:32, 33:64, 65:100, 101:200, 201:500, 501:1000 without gaps/overlap.
Their PNG-height sum tests additive per-row raster sizes and estimates a possible
extra pixel per capture. It does not construct a stitched PNG or assume +1 is universal.
Individual samples include rows 1, 2, 16, 32, 33, 64, 128, 512 and 1000.

A separate 28-height sweep around 12 and 12.75 points, plus heights near 15.75, compares
floor/nearest/ceil candidate models. Candidate predictions are labelled, never substituted
for measured PNG dimensions. The probe arrays and raw snapshots are the evidence; fitted
coefficients are not taken as universal conversion constants.

## Interpreting the data

`analysis.row_height.snapshots` contains raw rows and aggregate reads with phase labels.
`matrix[].summary` reports observed contrasts (large/small font, styled blank/unformatted
blank, variation under a fixed-height policy). It does not force a minimum-height verdict.
`geometry[].png_height_timeline`, `individual_row_changes`, and `paired_order_comparisons`
expose timing/order effects. Part-additivity residuals are tested at possible outside
pixel counts 0, 1, and 2. PNGs are preserved to inspect text clipping: a short height alone
is not automatic proof that a particular glyph was clipped.

Image/geometry errors stop the affected causal sequence rather than retrying in a
changed Excel state and silently calling that the original observation. A timeout does
not cancel an Office request; subsequent Office calls are prohibited. All saved results
are plain detached data, not live Office proxies. Progress is checkpointed after each
snapshot and image. Existing run-ID protection and per-workbook ownership are reused.

## Development tests

```bash
node docs/visual-validation/experiments/range-image-calibration/tests/diagnostics.test.cjs
node docs/visual-validation/experiments/range-image-calibration/tests/row-height.test.cjs
python docs/visual-validation/experiments/range-image-calibration/tests/test_row_height_fixtures.py
```

These test package structure, the intended isolated factors, JS orchestration with
mocked Office calls, read order and timeout handling. They do NOT execute real Excel,
Windows PowerShell/WPF or the ASP.NET build. Synthetic header mocks are not real PNG
rendering evidence.

## Primary references

- Row XML and its separate ht/customHeight attributes:
  https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.row
- Sheet-level format properties:
  https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.sheetformatproperties
- Range width/height and getImage:
  https://learn.microsoft.com/en-us/javascript/api/excel/excel.range
- RangeFormat.rowHeight and explicit autofitRows:
  https://learn.microsoft.com/en-us/javascript/api/excel/excel.rangeformat
- Microsoft support distinguishes specific row height from AutoFit, including clipped wrapped text:
  https://support.microsoft.com/en-us/excel/wrap-text-in-a-cell-in-excel

These references explain the available properties. They are not a specification of the
exact Excel renderer rounding or proof of a general defaultRowHeight-as-minimum rule.
