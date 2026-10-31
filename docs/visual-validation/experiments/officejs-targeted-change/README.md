# Targeted captures after a worksheet byte scan

This isolated experiment starts with **zero baseline image tiles**. It searches
two XLSX files for changed cells, then captures only a one-cell neighborhood
around each change and the complete bounds of one chart and one drawing. It
does not change the production hash sweep.

## Fixture and method

[`run.py`](run.py) reuses the numeric rainbow generator from the
[image-limit experiment](../officejs-image-limit/README.md). The resulting
`RGB sweep` sheet has 100,000 populated rows in column A. Excel adds an
`Objects` sheet with a line chart sourced from `RGB sweep!A1:A16` and a static
rectangle. The chart and rectangle occupy separate cell ranges.

The runner copies the Excel-saved baseline, then [`sheet_bytes.py`](sheet_bytes.py)
streams `sheet2.xml` one row at a time. Its row/cell state machine changes
`A5` from `5` to `50000` and removes the entire `A50000` cell. It also merges
two sorted byte-scanned cell streams to find changed coordinates without a
worksheet DOM or per-cell map. Excel opens, recalculates and saves the variant;
the scanner repeats the comparison afterward. Excel renumbers style IDs when
it removes a style that became unused, so the comparison resolves each ID to
its actual fill color. The small style, chart and drawing metadata parts are
read separately.

An injected Office.js diagnosis operation reads object positions and calls
`Range.getImage()` for `A4:B6`, `A49999:B50001`, the full chart range `A5:H20`
and the full drawing range `I6:K13`. The same four ranges are captured in each
workbook. The two cell ranges are derived from the scanner's output coordinates.
PNGs are saved and SHA-256 hashes are checked against the add-in's hashes. The
chart source and its cached fifth value are checked in chart XML.

Run from Windows Python with an available Windows `excel-shrink-cli.exe`:

```powershell
python docs/visual-validation/experiments/officejs-targeted-change/run.py `
  --binary C:\path\to\excel-shrink-cli.exe
```

`--rows 100 --changed-row 50` makes a quick smoke fixture. The default uses
100,000 rows and removes `A50000`. The workbooks and full telemetry remain in
Windows TEMP. The eight small PNGs and the compact
[result JSON](results/targeted-100k.json) are retained here.

## Result on 2026-10-31

The scan found exactly `A5` and `A50000` before and after Excel recalculated
the changed copy. Excel's chart cache changed from `5` to `50000` at its fifth
point. The XML drawing anchors, Excel's object cell bounds and Office.js object
positions agreed for this chart and rectangle. Office.js lists the chart in
both `worksheet.charts` and `worksheet.shapes`.

| Capture | Baseline | Changed copy | SHA-256 result |
| --- | --- | --- | --- |
| `A4:B6` | [PNG](results/images/baseline/changed-value.png) | [PNG](results/images/variant/changed-value.png) | Different |
| `A49999:B50001` | [PNG](results/images/baseline/removed-cell.png) | [PNG](results/images/variant/removed-cell.png) | Different; the number and fill disappeared |
| Chart `A5:H20` | [PNG](results/images/baseline/chart.png) | [PNG](results/images/variant/chart.png) | Different; the line has a visible spike |
| Drawing `I6:K13` | [PNG](results/images/baseline/drawing.png) | [PNG](results/images/variant/drawing.png) | Identical |

| Measured step | Time |
| --- | ---: |
| Excel adds objects, recalculates and saves baseline | 4.89 s |
| Stream-copy and change two cells | 1.20 s |
| Byte scan before recalculation | 1.83 s |
| Excel recalculates and saves variant | 4.23 s |
| Byte scan after recalculation | 1.75 s |
| Open baseline in resident Excel and ready add-in | 7.34 s |
| Open variant in resident Excel and ready add-in | 15.41 s |
| Eight `getImage()` calls combined | 0.19 s |

The largest individual image call was the baseline chart at 61 ms. The
resident Excel process peaked at **288 MB working set** across the two capture
runs; this excludes WebView2 and the temporary Excel instances used to prepare
the files. Each diagnosis command also has transport and polling overhead,
which is included in the [JSON](results/targeted-100k.json) but not in the
`getImage()` sum.

The workbooks can be opened from
`C:\Users\aue-johr\AppData\Local\Temp\officejs-targeted-change-1793460327379713800`.

## What the result establishes

For these **two simple two-cell OOXML anchors**, the entire chart and drawing
could be bounded, and the selected images detected the intended visual changes.
The current production drawing parser finds worksheet-wide maximum extents for
such anchors; it does not yet return an individual two-dimensional capture
rectangle per object. This experiment's chart selection uses its known source
formula and Excel's measured object bounds. It does not demonstrate a general
dependency parser for arbitrary charts, formulas, conditional formatting or
text overflow beyond the one-cell halo. A scanner unable to bound an object or
dependency would need to report that uncertainty instead of declaring skipped
areas visually equal.
