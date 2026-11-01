# Standalone chart-sheet access

## Question and result

Can standalone chart sheets be compared through Office.js, or without launching
an additional COM Excel instance?

Tested on this Windows/Excel installation on 2026-11-01:

- Office.js lists the embedded worksheet chart. Addressing `Standalone` through
  the worksheet API fails with `UnsupportedSheet`: “This sheet type does not
  support this operation, since it is a Macro or Chart sheet.”
- The existing PID-validated `CaptureErrorChecking.GetOwnedApplication()` bridge
  can borrow the **already running native resident**. No `New-Object -ComObject
  Excel.Application`, new workbook, or extra Excel instance is used for export.
- `Workbook.Charts` exposes both standalone chart sheets.
- `Chart.Export(..., 'PNG', false)` initially produced empty files despite a
  positive return value. Activating and refreshing the chart, then allowing
  200 ms for rendering, produced real PNGs. This delay is an experimental value,
  not an established universal readiness guarantee.
- Two exports of `Standalone` were pixel-identical at 1995 × 1304. Exporting
  `Different` changed the title region, bounding box `(934, 28, 1060, 47)`.
- Office.js still answered after exports, the source file hash was unchanged,
  and resident close confirmed Excel exit. Successful local results are under
  `.tmp/chartsheet-access-3/` and are not committed.

## Reproduce

Requires the existing debug runner, Windows desktop Excel, Python, `openpyxl`
and Pillow. Uses a private desktop; the interactive user's Excel is untouched.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/chartsheet-access/run.ps1 -Output .tmp/chartsheet-access-new
```

The fixture has a normal worksheet, one embedded chart and two chart sheets.
The driver first probes Office.js, then exports chart sheets through the owned
process bridge. `verify.py` checks decoded pixels, post-export Office.js access,
source preservation and close confirmation. The parent bounds the experiment
with an owned process job and a 150-second deadline.

## Recommended integration

Keep Office.js for embedded charts. Add a narrowly scoped chart-sheet fallback
to the same resident host, borrowing its owned application only for inventory
and export. Use the existing chart comparison/reporting for the resulting pixels.
Keep pre-rebuild and rebuild-copy exports separate, exactly like embedded charts.

Before production use, preserve/restore the active sheet, keep export viewport
dimensions consistent across states, validate decoded image dimensions and reject
empty exports even when Excel returns true. Any readiness retries must remain
bounded by the existing operation deadline. Do not move chart sheets into normal
worksheets or save them merely to make Office.js access possible: that changes
the state being validated. The production unsupported-chart-sheet guard remains
in place; this experiment does not silently change corpus coverage.

## Primary sources

- [Office.js chart collection](https://learn.microsoft.com/en-us/javascript/api/excel/excel.chartcollection)
- [Office.js unsupported-sheet issue](https://github.com/OfficeDev/office-js/issues/2444)
- [Excel Chart.Export](https://learn.microsoft.com/en-us/office/vba/api/excel.chart.export)
