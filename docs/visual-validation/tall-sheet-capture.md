# Capture coverage for unusually tall sheets

Resident corpus comparisons now shorten unusually tall sheets by default. Small
sheets retain their existing complete capture coverage. This is an explicit
sampling policy, not a claim that unrendered cells are visually identical.
The existing whole-workbook package-equivalence shortcut is unchanged.

## Configuration

The WPF corpus launcher and Observatory expose two settings:

| Setting | Default | Meaning |
| --- | --- | --- |
| Tall-sheet threshold | 40 | Shortening is considered only when a sheet needs **more than** this many capture bands. |
| Capture bands at each end | 2 | Always retain this many bands at the top **and** bottom, plus all indicator-selected bands. |

The threshold uses the actual common row bands negotiated after Excel measures
the original and profile sheets. It does not use file size, worksheet row count,
or the number of smaller inspection tiles displayed inside a captured image.
The current capture bands span the sheet width. Changing the capture pixel
budget can therefore change the number of bands and whether shortening applies.

For example:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-corpus.ps1 -Headless -Corpus Data.gov -TallSheetThreshold 40 -TallSheetEdgeTiles 2
```

Both values must be positive integers. Observatory changes apply to subsequent
groups; an admitted group's selection settings remain fixed. There is no
separate comparison-mode selector.

## Which middle bands remain

For a tall sheet, the runner retains the union of the edge samples and bands
intersecting any of these indicators across the original and profile variants:

- Changed, inserted, or removed worksheet XML rows.
- Conditional-formatting targets, including extended Excel `sqref` ranges.
- Decorated cells, rows, columns, and merged regions. Number/date formats alone
  do not select every row; fills, borders, fonts, and alignment can do so.
- Drawing and chart anchors from both versions, including their old positions.
- Strict cell-value differences found using saved recalculation copies.
- Differences in the row heights actually measured by Excel.

Deleted areas remain represented by the original geometry. Conditional-formatting
targets are not clipped to populated cells. Unknown layout changes, unsupported
objects or extensions, changed global styles/shared strings, missing analysis
inputs, and failed recalculation cause conservative full capture. Worksheet
indicators are read directly from the decompression stream, one row or other
worksheet child at a time. Total worksheet XML size is not limited to 256 MiB.
A single element or accumulated geometry metadata exceeding 8 MiB retains the
explicit full-capture fallback. Global parts such as styles and shared strings
still have the existing 256 MiB limit.

Unlike the earlier proposed blanket fallback, deleting a hidden or whitespace
value does not by itself force every sheet in the workbook to be captured.
Small sheets are already complete; tall-sheet selection uses the indicators
above and their dependent recalculated value changes.

## Recalculation and original screenshots

Existing profile recalculation copies from value validation are reused. If the
original reference is missing, one additional original recalculation may be
needed. This preflight happens **before** resident capture clients open, avoiding
isolated Excel cleanup while their shared WebView processes are active. Its
early eligibility check is conservative because exact band counts are not yet
available.

For the tall sheets, strict value comparison covers original versus recalculated
original, each profile versus its recalculated copy, and the paired recalculated
original/profile. These changes only add capture candidates. Changes also seen
when recalculating the original are recorded, never subtracted or automatically
excused as volatile formulas.

The screenshot clients still open the original input and ordinary Shrink
outputs. Saved recalculation copies identify places to inspect; they do not
replace the visual comparison inputs.

## Observatory, reports, and resume

Omitted bands appear as compressed gaps with break marks and explanatory
tooltips. They are marked **not visually checked**, separately from pending
captures, matching pixels, and visual differences. Gap counts refer to actual
capture bands, not displayed inspection subdivisions.

Each sheet's `capture_coverage` records policy `tall-sheet-v1`, the settings,
total/selected/omitted band counts, omitted row ranges, indicator reasons,
fallback reasons, and recalculation provenance. The resident session also writes
`capture-coverage.json`. A comparison with omissions reports that the checked
ranges were unchanged, rather than asserting complete visual equality.

Existing completed records remain intact. Resume does not repeat them merely
because this policy or its threshold changed. New records preserve their own
coverage through normal report/export/import handling; older records are not
retroactively labelled as sampled.

## Observed Lysimeter example

The existing workbook group `ff19952ef82267eebfff` illustrates both outcomes.
Its `1990 E Maize Lys 15-Min` sheet had 1,062 planned bands; static XML analysis
found only one decorated header row before adding edge, recalculation, and
measured-height candidates.

Its `1990 E Maize Lys 5-Min` sheet does contain conditional formatting, including
targets `J70953:J1048576` and `H55417:H1048576` in the original and all three
profiles. Those long tails must remain capture candidates under this policy.
A tall, mostly numeric appearance alone is therefore insufficient to justify
omission.

## Bounded native check

`excel-shrink-cli/office-capture/tests/tall-sheet-smoke.py` creates a small synthetic
workbook with a tall data sheet, conditional formatting, a filled area, hidden
data, whitespace and a small formula sheet. Python with `openpyxl` is required.
Without `--run` it only prepares the fixture. With `--run` it starts isolated,
visible Excel instances and checks coverage plus byte-identical export/import.

```powershell
python excel-shrink-cli/office-capture/tests/tall-sheet-smoke.py --run --port 8799
python excel-shrink-cli/office-capture/tests/tall-sheet-smoke.py --run --reference --port 8800 --compare-to .tmp/tall-sheet-smoke/<sampled-run-directory>
```

The reference uses a high threshold, not a separate production mode. The initial
native check retained 5 of 89 bands on the tall sheet and the entire small sheet.
The reference captured all 89 tall-sheet bands; both runs found the same visual
differences (none for this fixture). This checks the pipeline, not a claim of
complete coverage or a general performance guarantee for sampled workbooks.
