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

## Wide Base62 rainbow: image and hash cost

[`entropy_probe.py`](entropy_probe.py) creates a single sheet with a value in
every cell. The labels use Base62 (`0-9`, `a-z`, `A-Z`); each label's integer
value also chooses its fill from the 1,530-step RGB rainbow. Adjacent cells
change one color channel by one unit, horizontally and vertically. The sheet
uses 5-point text, 8-point rows and narrow columns. These are ordinary cell
fills, not conditional formatting. The image is visually busy but still
compressible; it does not model random pixels or the worst possible PNG size.

The script injects [`entropy-operation.js`](entropy-operation.js) into a
disposable resident copy, opens its own Excel instance, measures cell pixels,
then requests an image wider than the known limit. It searches the largest
whole-column width that returns completely, captures that width at the chosen
height, hashes the PNG in Office.js and saves the PNG for inspection. All
generated files stay under Windows TEMP, outside Git.

```powershell
python docs/visual-validation/experiments/officejs-image-limit/entropy_probe.py `
  --run --rows 640 --binary C:\path\to\excel-shrink-cli.exe
```

Without `--run`, it creates only the workbook. The default is 2,600 columns
by 320 rows; `--rows 160` and `--rows 640` reproduce the other measured sizes.
The compact [result JSON](results/base62-rainbow-width.json) records the input
hashes, PNG hashes, times, dimensions and Windows TEMP paths.

### Result on 2026-10-31

Excel rendered each cell 14 px wide and 10 px high. A request for 2,600
columns should have been **36,401 px** wide; `getImage()` returned success but
silently clipped it to **32,766 px**. With 2,340 columns, the complete image
was **32,761 px** wide. The next column would require 32,775 px and was clipped.
The same boundary appeared in all three runs.

| Rows | Complete image | Pixels | `getImage()` | PNG on disk | SHA-256 | RGBA bitmap |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 160 | 32,761 × 1,601 | 52.5 MP | 11.0 s | 15.9 MB | 18 ms | 210 MB |
| 320 | 32,761 × 3,201 | 104.9 MP | 23.7 s | 32.8 MB | 38 ms | 419 MB |
| 640 | 32,761 × 6,401 | 209.7 MP | 51.7 s | 66.1 MB | 83 ms | 839 MB |

The 640-row PNG is
`C:\Users\aue-johr\AppData\Local\Temp\officejs-entropy-probe-1793456618549578200\maximum-complete.png`.
Its workbook is `base62-rainbow.xlsx` in the same directory. Pixel samples at
the beginning and far edge of the 640-row PNG matched the expected fill colors;
the saved PNG hashes also match the add-in's hashes. The 640-row PNG's SHA-256
is `649380ec6424592ed5a79816b83fd8635dee3902f1bd0575e0458c539133c651`.

The monitored Excel process peaked at about **1.30 GB working set** in the
640-row run. That is a peak for the entire experiment, not memory attributable
only to its last image, and it excludes WebView2 and other processes. The
mathematical RGBA size is one pixel buffer, not a bound on transient Excel,
JavaScript, base64 or PNG buffers. A 209.7-MP image succeeded here; larger
images, other content and other Excel installations remain untested. The
production worker currently uses a 30,000-px edge and at most 64 MP per image.

### Near 32,000 × 32,000 pixels

The same fixture was expanded to 2,286 columns and 3,200 rows. At the measured
cell size, one full range would be **32,005 × 32,001 px**, or **1.024 billion
pixels**. This run hashes the returned PNG without sending its base64 data
through the resident JSON channel; it does not save a PNG. Thus the measured
PNG byte length describes its potential disk size, but only the smaller runs
above provide images to inspect.

```powershell
python docs/visual-validation/experiments/officejs-image-limit/entropy_probe.py `
  --run --single-capture --columns 2286 --rows 3200 --timeout 900 `
  --binary C:\path\to\excel-shrink-cli.exe
```

| Range | Expected pixels | Result | Elapsed / `getImage()` | PNG bytes | Excel peak working set |
| --- | ---: | --- | ---: | ---: | ---: |
| 2,286 × 1,600 cells | 512.1 MP | Complete, 32,005 × 16,001 px | 193.4 / 192.0 s | 157.8 MB | 2.77 GB |
| 2,286 × 3,200 cells | 1,024.2 MP | Office.js `GeneralException`, no image | 104.1 s / unknown | unavailable | 1.46 GB |

The successful 512-MP image has SHA-256
`71dc414594e64e83065fa350936e29961664b0c44b4313df6f517bc24b7ad12e`;
hashing its PNG took 182 ms. Its single RGBA bitmap would occupy **2.05 GB**.
The failed request would require **4.10 GB** for one RGBA bitmap. Office.js
reported “Interner Fehler während der Verarbeitung der Anforderung.” after
104 seconds. Excel stayed open and returned the error through the add-in; the
failure did not supply image dimensions, PNG bytes or a hash. Windows reported
13.3 GB free physical RAM before the 1.024-GP run, and the monitored Excel
process did not exhaust that amount. The measurements do not cover WebView2
or native image buffers, so they do not identify the internal cause.

These two probes show that this installation can return a 512-MP image but
failed at 1.024 GP for this fixture. They do not establish a portable pixel
limit or a safe production setting. The [result JSON](results/base62-rainbow-width.json)
also records the two workbook hashes and their temporary paths.
