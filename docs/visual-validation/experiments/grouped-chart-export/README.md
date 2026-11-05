# Grouped chart export

Observed on Windows Excel on 2026-11-05 with
`Glassmeyer et al Geo Health Figures _e04dac9ee465.xlsx`.

## Reproduced failure

The Office.js chart collection includes `Figure 4`, `Chart 1`, index 1.
Requesting its image returns `ItemNotFound` after approximately 40 milliseconds.
The existing native fallback obtains the workbook and chart successfully.
Instrumentation reaches `Chart.Export` after completing activation and refresh,
then the command exceeds its 180-second deadline. Owned process cleanup completes.

The drawing XML places this chart inside `Group 4`, alongside `Picture 2`.
The original isolated probe and operation trace are retained on V under
`.tmp/validation-temp/glassmeyer-chart-probe/`.

## Successful containing-group probe

An isolated worker exported the known containing group with Office.js
`Shape.getAsImage('PNG')`. Its image contains the scatter plot, axes, labels,
legend and data points. The complete native comparison finished in 46.948 seconds.
Each rebuilt state supplied one worksheet region and 29 chart observations.
There were zero reported differences, and both Excel processes closed.

The grouped observation records its group name, placement and
`export_method=office-js-containing-group`. The image covers the containing
group, so its extent includes other children in that group.

Evidence is retained under `.tmp/validation-temp/glassmeyer-shape-proof/` on V.

| Artifact | SHA-256 |
| --- | --- |
| Original input | `e04dac9ee4657f37823c213d1a22d28d6c3a66e11ed6a92c9394314fbbb540af` |
| Default input | `fe2ba8736789f7575b067576c13df404ca24a5e49a12a8e4dae98005de64c7c2` |
| `result/report.json` | `5ce96e3059144c54c7337a186eeb8a24fb47e2b21e8443d2a8f676a26998470d` |

## Remaining verification

The successful probe uses the known group name. A separate discovery prototype
walks group descendants, requires a unique chart-named unsupported shape and
renders its top-level containing group. Twelve JavaScript tests pass, including
nested groups, ambiguous matches, ordinary-image name collisions and existing
chart export behavior. Native verification of this discovery step is outstanding.
The prototype remains in the isolated experiment directory.

The supported image API and group traversal are documented in
[Excel.Shape](https://learn.microsoft.com/en-us/javascript/api/excel/excel.shape?view=excel-js-preview)
and [Excel.ShapeGroup](https://learn.microsoft.com/en-us/javascript/api/excel/excel.shapegroup?view=excel-js-preview).
