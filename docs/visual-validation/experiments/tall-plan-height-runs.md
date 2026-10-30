# Reusing Excel's measured row-height runs during capture planning

The resident already obtains compact, verified row-height runs for visual
comparison. Previously, `SheetGeometry.read()` then asked Office.js for every
row height again before splitting the sheet into capture tiles. Reusing those
runs avoids the duplicate Excel calls. Tile splitting itself remains local and
keeps the existing full-width ranges, image limits, and paired row boundaries.

## Lysimeter measurement

The original Data.gov workbook
`weighing-lysimeter-data-for-the-bushland-texas-maize-for-grain-datasets__2c889f5a-95c3-4f68-905a-672529331db0_3b8b2de83a3e.xlsx`,
sheet `1990 E Maize Lys 5-Min`, has 105,121 populated rows. Both measurements
used one isolated Windows Excel client and a 32 MP tile budget. Times exclude
workbook startup.

| Workbook and planning path | One-sheet command time | Planned tiles |
| --- | ---: | ---: |
| Original, read every row through Office.js again | 23.405 s | 263 |
| Original, reuse measured height runs | 0.541 s; 1.600 s on repeat | 263 |
| Default Shrink output, reuse measured height runs | 1.120 s; 0.743 s on repeat | 263 |

For both original and Default output, every planned range, core-row interval,
width, height, capture-pixel count and right boundary matched its earlier plan.
The original's new command reported 0.164 s for
the existing row-height run measurement, 0.036 s for geometry construction,
0.069 s for image calibration and 0.004 s for tile splitting. Other workbook
checks and command overhead account for the remainder. These are two separate
runs on the same machine, so the time difference is evidence for this workbook,
not a general performance guarantee. The prior Default timing was recorded only
for a two-sheet command, so it is not used as a one-sheet speed comparison.

A separate mixed-height check used visible rows of different heights and a
hidden row. It produced identical row geometry and eight tile boundaries while
reducing range requests from 10,016 to 16 in that model. When a cached
geometry must be rebuilt, the worker measures the height runs again and rejects
a changed layout before using them.

The repeat runs also captured and hashed tiles 0, 131 and 262 on both sides.
Their actual PNG dimensions matched the plan. Each matching original/Default
pair had the same SHA-256 hash, and the retained PNGs were released without
being saved as experiment artifacts. This checks representative rendered tiles;
it is not a complete visual comparison of the sheet.

The result does not justify introducing a second sparse tile planner yet: the
local tile split took only 4.4 ms. Selection still decides which of the 263
planned tiles to capture. A future preflight could avoid opening Excel at all
for sheets explicitly reported as unsupported, but that is a separate policy.
