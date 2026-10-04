# PNG units and marker diagnostics

> Current runs: see [consolidated capture/report contract](../CONSOLIDATED-CAPTURE.md). Output is now central
> at `docs/visual-validation/results/`. Historical paths and monolithic timing notes below
> describe the earlier experiments, not the current safe range-image capture path.


These tests isolate raw OOXML geometry, Office.js geometry and actual PNG output. They
are engineering diagnostics, not production capture changes. Synthetic input packages are
assembled from minimal XML with Python's standard-library XML/ZIP tools. No source hospital
data, external links, macros, formula dependencies, merged cells or drawings are copied in.
The supplied incomplete unzipped hospital export was used only as a structure/geometry example.

## Distinguish the units

- `row@ht` and `sheetFormatPr@defaultRowHeight` are point-based heights.
- Stored `col@width` is a character-based measure related to the maximum digit width of the
  Normal-style font; it is not the same unit as row height or Office.js Range.width.
- Office.js `Range.width` and `Range.height` both report points at 100% zoom.
- PNG IHDR dimensions are output pixels. `points * 96 / 72` is a 96-DPI hypothesis, not an
  observation. The experiment separately fits x/y output slopes **and intercepts**.

The documented stored-column-width conversion tested against the observed geometry is:

```
px = floor(((256 * stored_xml_width + floor(128 / maximum_digit_width)) / 256)
           * maximum_digit_width)
```

The five-pixel padding belongs to the conversion from desired character count to the
stored width. It must not blindly be added again to that stored-width-to-pixels formula.
The experiment compares candidate digit widths 1..32 pixels at a stated 96-DPI reference;
it does not assume that every workbook uses Calibri 11 with a 7-pixel digit width.
Small widths and quantization can produce residuals; candidate fitting is a diagnostic,
not a universal font renderer. Logged font names do not prove absence of font substitution.

## Four deterministic XLSX fixtures

The patch creates the packages and their SHA-256 manifest under `fixtures/`.
`generate-fixtures.py` is also installed for reproducible regeneration.

### Units: three Normal-style fonts

`vv-render-calibri11.xlsx`, `vv-render-arial10.xlsx`, `vv-render-calibri16.xlsx` have identical
raw geometry but different workbook Normal fonts. This changes the font used for the column
width unit, rather than just changing the formatting of an arbitrary cell.

Each contains:

- `Units`: 13 stored column widths and 11 explicit row heights, individual and cumulative
  captures, including 12.6, 12.75 and 15.95 point rows and quantized column widths.
- `Zoom150`, `GridOn`, `Borders`: otherwise identical geometry, varying one view/border setting.
  The appropriate synthetic worksheet is activated before capturing. Compare returned PNG
  dimensions rather than assuming that a stored zoom necessarily changes getImage resolution.
- `Fractional` and `Defaults`: 1/10/100/1000-row captures, explicit heights versus sheet defaults,
  to detect accumulation of per-row rounding rather than a simple global conversion factor.
- `Hidden`: internal hidden rows and columns, with the expected visible XML extent recorded.

Output includes `analysis.unit_conversion`, `analysis.view_controls`,
`analysis.fractional_and_default_geometry`, and `analysis.hidden_geometry`.
All raw attempts remain available in `probes` and CSV. Compare the three workbook results
to assess Normal-font dependence; no result is prefilled with expected empirical factors.

### Markers: clipping versus compression, dimensions versus area

`vv-render-markers.xlsx` contains:

- `Tall`: 4096 rows at 12.75 points, 16 columns, twelve distinct solid-color four-row bands.
- `Wide`: four rows, 800 columns, twelve distinct four-column bands.

The diagnostic first searches for a plateau on a narrow A:D vertical strip. It then compares
A:A and A:D at the same full height, and A:P as a wider header-only control. A horizontal
plateau search tests the complementary small-height, large-width case. This avoids using
hundreds of millions of pixels just to test a single long dimension.

The marker ranges are measured using Office.js `top/left/height/width`. Raw PNGs from selected
narrow captures are saved and decoded **on the .NET host**, not through a browser canvas.
A small below-cap control first checks that the band detector sees the expected colors.
Color centers in the capped image are compared with both native-scale and compressed-scale
predictions. Classification is conservative: `consistent-with-clipping`,
`consistent-with-scaling-on-tested-axis`, or `inconclusive`. Missing last bands alone are not
sufficient; earlier band positions must also agree with the relevant geometric model.

The host identifies dimensions before decoding and rejects marker images over 20 million
pixels. This decode safety budget is separate from the measured Excel behavior. The original
Base64/PNG is never re-encoded by a canvas. Analysis/HTTP failure is recorded separately from
Range.getImage success. Far-down/far-right small captures test absolute coordinate versus
captured extent. Marker results on synthetic sheets do not prove the content of the hospital
capture without a corresponding content check there.

## Run

**Experiment: visual validation -> GUI -> Get picture units and markers** offers three suites:
all, units, and markers. No hospital workbook is needed for this experiment.

```powershell
.\docs\visual-validation\experiments\range-image-calibration\run.ps1 -Suite units
.\docs\visual-validation\experiments\range-image-calibration\run.ps1 -Suite markers
.\docs\visual-validation\experiments\range-image-calibration\run.ps1 -Suite all
```

Optional: `-WaitForStart -BreakOnError -UseOfficeDebugJs`.
These modes intentionally use the existing ASP.NET/.NET host and an isolated Excel /x
process; the Rust host and the A-F screenshot pipeline are unchanged.
Fixtures are SHA-checked before launch; the existing runner opens disposable copies.

Latest output is `results/officejs-dotnet-calibration.json` and a sibling CSV.
Timestamped runs, raw marker PNGs, progress snapshots and host logs are archived under
`.tmp/visual-validation/range-image-calibration/runs/<UTC-time-and-id>/`.
Each workbook result includes Excel/Office diagnostics, the EXCEL.EXE file version, source
hash, requested font/geometry, browser user agent and devicePixelRatio. Browser DPR is
recorded as context, not assumed to be the PNG's DPI. Stale-pane writes use a run-ID guard.
A timeout is not treated as a dimension cap. Inspect `status` and individual conclusions.

## Tests supplied with the patch

Run from the repository root:

```text
node docs/visual-validation/experiments/range-image-calibration/tests/diagnostics.test.cjs
python docs/visual-validation/experiments/range-image-calibration/tests/test_fixtures.py
```

These validate algorithms, mocked JavaScript integration, deterministic generation and
package/relationship integrity. They do NOT substitute for running real Excel, WPF,
Windows PowerShell and the ASP.NET host on Windows. The tiny PNGs in Node tests are
header-only mocks and must not be interpreted as real rendered samples.

## Sources and status (checked 2026-10-04)

- Microsoft Open XML Column remarks (stored width, Normal font, padding and conversion):
  https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.column?view=openxml-3.0.1
- Microsoft Open XML Row:
  https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.row?view=openxml-3.0.1
- Microsoft Support, Change the column width and row height:
  https://support.microsoft.com/en-us/excel/change-the-column-width-and-row-height
- Microsoft Office.js Range reference (width/height points at 100% zoom; getImage Base64 PNG):
  https://learn.microsoft.com/en-us/javascript/api/excel/excel.range?view=excel-js-preview
- W3C PNG specification, IHDR and integer ranges:
  https://www.w3.org/TR/png-3/

PNG permits dimensions up to 2^31-1, far beyond 32766. No explicit 32766/32767 getImage
maximum was found in the cited Microsoft method documentation. A signed-16-bit-related
internal cap is a hypothesis prompted by the user's output, not a documented implementation
fact. The new search learns a plateau from returned pixels instead of encoding that hypothesis
as a pass/fail threshold.

## Additional row-height follow-up

The separate `row-heights` suite tests cell fonts, inherited fonts, sheet/row
`customHeight`, geometry-read order and fixed-row pixel rounding. It creates its own
latest results and a review ZIP; `all`, `units`, and `markers` keep their original scope.
See [ROW-HEIGHT-FOLLOWUP.md](ROW-HEIGHT-FOLLOWUP.md) for running and returning results.
