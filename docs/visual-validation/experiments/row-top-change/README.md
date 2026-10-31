# Empty font cell changes effective row height

This isolated experiment tests a row-height change caused by removing an
empty cell with a unique large font. It does not change the production capture
planner.

Run on Windows with Excel and Python installed:

```powershell
powershell -ExecutionPolicy Bypass -File docs/visual-validation/experiments/row-top-change/run.ps1
```

[`run.ps1`](run.ps1) asks Excel to create a sheet with content in rows 1 and
1,000,000 and an empty, 40-point-font cell at `XFD500000`.
[`make_variants.py`](make_variants.py) fills row 500,000 with empty XML cell
nodes across all 16,384 Excel columns. The changed copy omits only the
`XFD500000` node. Both copies retain the same XML row tag, including its
`ht="53.25"` attribute. Excel opens each copy read-only; the script measures
the effective row height and `Top` coordinates, then searches for the first
displaced row. Workbooks and detailed output are written to
`%TEMP%\excel-shrink-row-top-change`.

To inspect what Excel writes after recalculation and saving, run
[`recalc-and-save.ps1`](recalc-and-save.ps1) after `run.ps1`. It uses a
separate copy, leaving the comparison files intact.

## Result on 2026-10-31

| Measurement | Original | Cell removed |
| --- | ---: | ---: |
| Empty cells in row 500,000 | 16,384 | 16,383 |
| Effective height of row 500,000 | 53.25 points | 15 points |
| `Top` of row 500,000 | 7,499,985 points | 7,499,985 points |
| `Top` of row 500,001 | 7,500,038.25 points | 7,500,000 points |
| `Top` of row 1,000,000 | 15,000,023.25 points | 14,999,985 points |

The first displaced row was **500,001**. The 20-query binary search took
**230 ms**. Before either variant is saved again, both XML files say
`ht="53.25"`; Excel nevertheless uses different effective heights after
loading them. The stored row-height attribute alone cannot settle this case.
After `CalculateFullRebuild()` and saving the changed copy, Excel removes
the now entirely empty row 500,000 from its sheet XML, including `ht`.
The saved XML was inspected only after both recalculation and saving. The
compact [result JSON](results/2026-10-31.json) records the measurements.

The binary search works for this fixture's single height change. If several
height changes cancel, equal `Top` values farther down cannot rule out local
changes. In that case, inspect the effective height near each candidate row
identified by the XML difference. Here the removed cell itself identifies
row 500,000 without scanning all million rows in Excel.

## Height query timing

On the same two files, a separate COM probe read `Rows(500000).RowHeight`
and two other rows 100 times each per workbook. A warm single call had a
median of **4.47 ms** for the original and **4.51 ms** for the changed copy.
The first query of the original took **243 ms**, reflecting initial layout
work. Opening these cached, small synthetic workbooks took 416 and 342 ms in
that probe; opening a large real corpus workbook can cost much more.

An isolated Office.js diagnosis worker queried `Range.height` and
`Range.format.rowHeight` 101 times per workbook, with one `context.sync()` per
query. The median warm sync took **1.7 ms** for the original and **3.1 ms**
for the changed copy; the first sync took 3.4 and 3.9 ms. All 101 queries
together took 213 and 345 ms inside the worker. Starting Excel and the add-in
took **9.6 and 7.6 seconds** in these two runs. A diagnosis command observed
from the host took about **2 seconds** because this test harness polls for
responses; that delay is not the Excel height query itself.

## Opening and saving controls

After `run.ps1`, run [`save-controls.ps1`](save-controls.ps1). It uses
[`make_opening_fixtures.py`](make_opening_fixtures.py) to add `=A2*2` with a
deliberately stale saved value of `-999`; the calculated value is `4`. It
checks manual calculation, automatic calculation, and automatic calculation
with `fullCalcOnLoad` enabled. Each case measures the open workbook before and
after `SaveCopyAs`, ordinary `Save`, and explicit Full Rebuild, then reopens
both saved outputs. The removed-font-cell workbook is also tested.

On this Excel installation, the stale value remained `-999` on opening in
both the manual and ordinary automatic cases. With `fullCalcOnLoad` enabled,
Excel returned `4` immediately on opening. Ordinary `Save` and `SaveCopyAs`
left the stale value unchanged in the manual case; explicit Full Rebuild
produced `4`.

For the removed-font-cell fixture, Excel reported 15-point height as soon as
it opened the file, although its row XML still said `ht="53.25"`. Both
`SaveCopyAs` and `Save` removed the now empty row from their XML. The open
workbook and both reopened outputs all reported 15-point height. This case
did **not** require a reopen to obtain the current visual geometry.
[`compare-open-and-reopened.ps1`](compare-open-and-reopened.ps1) also captured
`A499999:B500001` with visible text directly below the removed row. Its three
161 × 61 PNGs, taken before `Save`, after `Save`, and after reopening, were
byte-identical. The images remain in `%TEMP%\excel-shrink-row-top-change\save-image-control`.

Two small real Destatis copies were also saved in manual calculation mode.
`Save` and `SaveCopyAs` were not byte-identical. In
`foerderprogramme-5411206209005.xlsx`, `SaveCopyAs` stored `ht="13.9"` and
`ht="17.45"` for two rows, while `Save` stored `15` and `18`. Excel reported
effective heights of 15 and 18 points both in the open workbook and after
reopening either saved output. The difference in stored XML did not create a
different visible layout in this case. These measurements are in the
[save-control result JSON](results/save-controls-2026-10-31.json).

The controls establish that opening can recalculate when workbook settings
request it, and that `Save` and `SaveCopyAs` cannot be assumed identical.
They did not find a case where the open workbook rendered differently from a
saved and reopened copy. A corpus-wide guarantee would require broader tests.

After `Save` returned for the synthetic changed workbook, a separate process
also copied its XLSX while it remained open in Excel. The copy was a valid ZIP
and byte-identical to the saved file. This supports reading a completed
ordinary `Save` result without closing Excel on this machine; the runner must
still avoid reading during an active save and report sharing failures.
