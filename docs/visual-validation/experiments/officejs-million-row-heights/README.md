# Measuring every row height through Office.js

This experiment runs the unchanged production `CaptureGeometry.readHeightRuns`
on synthetic XLSX files with **1,048,576 populated numeric rows each**. Only a
temporary worker copy gains a timing operation. No production runner or corpus
configuration changes. Each workbook uses one isolated, hidden private-desktop
Excel client, closed before the next case starts.

## Run

Requires Windows, desktop Excel, Python and a built capture-host executable:

```powershell
python docs/visual-validation/experiments/officejs-million-row-heights/run.py `
  --output .tmp/million-row-height-new-run `
  --binary target/debug/excel-shrink-cli.exe
python docs/visual-validation/experiments/officejs-million-row-heights/verify.py `
  .tmp/million-row-height-new-run
```

Use a fresh output directory. Generated workbooks, worker copies, protocol
responses and full process metrics stay there. The checked-in
[compact result](results/2026-11-01.json) preserves timing and geometry evidence.

## Method

Python streams the worksheet XML directly into ZIP members. Column A contains
one explicitly default-styled numeric cell per row. Column B has no cell nodes
or values. The five cases are:

- Uniform default heights.
- Two 30-point rows (524,288 and 1,048,576), plus hidden row 8,192.
- Alternating explicit 15/30-point custom heights on every row.
- A 30-point font inherited from the entirely empty column B.
- The same workbook with only B's column-style attribute removed.

The last pair's normalized worksheet byte streams are independently hashed and
verified identical except for that attribute. The verifier also counts all XML
rows, checks the known outlier geometry and verifies recorded Excel exits.

The production scanner checks 8,192-row spans, up to 64 spans per Office.js
batch. It reads `top`, `height`, `rowHidden`, `rowCount` and `format.rowHeight`.
Only spans with consistent uniform geometry are accepted together; ambiguous
spans are split. The scan therefore covers every row without querying every row
individually in uniform cases. The timing wrapper counts actual batches/ranges
and verifies complete contiguous coverage. It does not replace the scanner.

Each complete case is measured twice in the same open workbook. Opening and
add-in readiness are separate from in-add-in measurement time. The first scan
is not a controlled cold-cache benchmark. Host wall times additionally contain
command delivery, the resident's startup quiet period and response observation.

## Results on 2026-11-01

Windows Excel **16.0.20527.20088**, one client at a time:

| Case | First scan | Repeat | Ranges / batches per scan | Result |
| --- | ---: | ---: | ---: | --- |
| Uniform | 668.5 ms | 58.6 ms | 128 / 2 | All rows: 15 pt |
| Three outliers | 410.4 ms | 399.2 ms | 206 / 15 | All three located correctly; six height runs |
| Alternating every row | Stopped at 30.03 s | Not repeated | 74,432 / 1,163 completed | Incomplete, explicitly reported |
| Empty B with 30 pt font | 72.0 ms | 109.2 ms | 128 / 2 | All rows: 39 pt |
| B's style removed | 676.6 ms | 75.5 ms | 128 / 2 | All rows: 15 pt |

Workbook/client startup took **6.53–7.53 seconds** per case. Peak Excel private
memory was **254–262 MiB**; this excludes WebView2 and the host. Every owned Excel
process was recorded closed. No images were captured or uploaded.

## Conclusions and limits

For uniform or sparsely varying heights, measuring all Excel rows was below
one second on this machine. A full-height check after column-format removal
can therefore be practical without complex row preselection in these cases.
The empty-column case confirms a real dependency: its inherited font changed
the height of every row despite the column having no values.

The alternating-height case is an important counterexample. This experiment
adds a 30-second measurement deadline, checked between Office.js batches, plus
an outer host response deadline. It stops without claiming complete coverage.
The existing scanner also has a 250,000-span request budget; a fully alternating
million-row sheet would exceed that budget even with more time. The protocol's
`ok: true` only means the diagnostic command returned; **`complete: false` is
not a successful height check**.

These are five synthetic cases with two scans each, not a corpus-wide timing
guarantee. The verification's 39-point font expectation records this Excel/font
environment. No formula/recalculation effects or image comparisons are tested.
Integrating the scanner into the targeted comparison, including an explicit
policy for expensive nonuniform cases, remains separate work.
