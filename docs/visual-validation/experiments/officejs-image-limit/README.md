# Office.js image height and tall-range hash sweep

This isolated experiment calls `Range.getImage()` directly. It does not change
the production capture budget. The related production smoke test
[`large-capture-smoke.py`](../../../../excel-shrink-cli/office-capture/tests/large-capture-smoke.py)
checks one image between 32 and 64 MP, not Excel's behavior on a much taller range.

## Fixture and method

[`run.py`](run.py) creates one XLSX sheet with a numeric value in every row.
Cell fills cycle through 1,530 RGB steps: one channel changes by at most one
unit per row, then the sequence repeats. This uses ordinary cell fills rather than
conditional formatting, because the question is whether `getImage()` returns
the complete pixels of a very tall range. A successful image should have one
20-pixel-high stripe per row and repeat the colors every 1,530 stripes.

The script copies the Office.js resident into a temporary sibling directory,
adds [`probe-operation.js`](probe-operation.js) there, and removes that copy at
the end. The operation directly requests `A1:A1048576`, then progressively
smaller ranges. The returned PNG dimensions are checked against the expected
height. A returned PNG with missing rows is a failed *complete* capture even
when Office.js reports no error. The saved PNGs permit visual inspection.

Run from Windows Python with an available Windows `excel-shrink-cli.exe`:

```powershell
python docs/visual-validation/experiments/officejs-image-limit/run.py --run --binary C:\path\to\excel-shrink-cli.exe
```

`--rows 1000` prepares a small fixture. Without `--run`, only the workbook is
created. `--existing-workbook` reuses a fixture; `--single-start-row` and
`--single-rows` request one range. The script writes the workbook, PNGs,
`results.json`, resident logs, and session telemetry under Windows TEMP.

## Result on 2026-10-30

On this Windows Excel installation, the full 1,048,576-row request returned
after **2.58 seconds**, without an Office.js error. Its PNG was only
**85 × 32,766 px**, while the 15-point rows require **20,971,521 px** of height.
The image contains the beginning of the requested range and is silently
truncated. The upper and lower half requests also returned 32,766-pixel-high
PNGs, after 0.55 and 2.97 seconds respectively. The lower-half image begins
with the expected color for row 524,289; it does not contain its whole range.

The boundary search found **1,638 rows complete** (85 × 32,761 px) and
**1,639 rows incomplete** (85 × 32,766 px). The full-image PNG and the
1,639-row PNG were byte-identical. Colors at rows 1, 2, 255, 256 and 1,531
match the generated cycle, including the expected restart at row 1,531.
Thus a successful `getImage()` response does not prove complete coverage;
dimensions must be checked. The production worker already validates planned
PNG dimensions and caps each edge at 30,000 px.

The resident reported Excel's peak private bytes at about **193 MB** during
the full request and peak working set at about **301 MB**. These counters cover
the monitored Excel process; they are not a total for WebView2 and the system.
CPU load was not measured. The elapsed times exclude workbook generation and
Excel startup.

This fixture models the *height* of the conditional-formatting tail in the
Data.gov workbook
`weighing-lysimeter-data-for-the-bushland-texas-maize-for-grain-datasets__2c889f5a-95c3-4f68-905a-672529331db0_3b8b2de83a3e.xlsx`,
sheet `1990 E Maize Lys 5-Min` (group `ff19952ef82267eebfff`). Its actual
formatting and column widths are different, so the numeric boundary measured
here should not be assumed to transfer to that workbook.

## Complete hash sweep of the real Lysimeter tail

[`hash_sweep.py`](hash_sweep.py) opens the original Data.gov workbook once and
captures `J70953:J1048576` in consecutive 1,700-row images. Each Office.js
PNG must be 65 px wide and contain every requested row. The temporary PNG
bytes are hashed with SHA-256 in the add-in and discarded. The script writes
only per-range hashes, dimensions, byte counts, timings and an ordered SHA-256
of the range/hash pairs to `manifest.json`, checkpointing after every 20 tiles.

```powershell
python docs/visual-validation/experiments/officejs-image-limit/hash_sweep.py `
  --workbook C:\path\to\original.xlsx `
  --binary C:\path\to\excel-shrink-cli.exe
```

On 2026-10-30, all **576 images** completed in **814.6 seconds (13 min 35 s)**
after Excel was ready. The saved manifest covers every row from 70,953 through
1,048,576 without a gap. The complete [hash manifest](results/lysimeter-j70953-j1048576.json)
is stored with this experiment. Its aggregate SHA-256 is
`82a440a8975177361a96bd7b4c535a152ad3313c2ee8084c34fc77f7c3111a28`.
The input workbook SHA-256 is
`3b8b2de83a3efe0b73b225143f1a3261b912f0dab7665497da934091a94937d1`.
No PNG files were saved. Excel's monitored peak working set was about 395 MB;
this excludes WebView2 and other process memory. The first 20 images averaged
0.30 s each; the final 16 averaged 2.89 s each. The run alone does not explain
why later ranges became slower.

This measures one column in **one original workbook**. It is not a visual
comparison between original and Shrink profiles, and its raw range PNG hashes
are not interchangeable with the production full-width, halo-cropped tile
hashes. Still, the measured cost matters for corpus planning: the intended
corpora contain roughly 20,000 Enron, 9,000 Data.gov, 1,000 Destatis and
5,000 EUSES files. We do not know how many have similarly expensive ranges.
As an illustration, if only 1% of those 35,000 files had one such range, a
single complete pass would consume about **79 hours of aggregate Excel capture
time** before comparisons against other profiles. Parallel workers can reduce
wall-clock time but do not remove that work. This is why the tall-sheet policy
selects ranges with evidence of possible changes and explicitly reports the
areas it did not visually check; see [tall-sheet-capture.md](../../tall-sheet-capture.md).
