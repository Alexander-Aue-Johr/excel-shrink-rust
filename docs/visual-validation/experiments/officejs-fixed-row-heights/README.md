# Fixed versus automatic row heights after removing empty formatting

This experiment tests whether unchanged explicit `ht` plus `customHeight="1"`
can exclude a row from automatic-height measurement when its empty font cells
or column font formatting are removed. It changes no production comparison
code and starts no corpus run.

## Run

Requires Windows, desktop Excel, Python and a built capture-host executable:

```powershell
python docs/visual-validation/experiments/officejs-fixed-row-heights/run.py `
  --output .tmp/fixed-height-new-run `
  --binary target/debug/excel-shrink-cli.exe
```

The script reuses package/protocol helpers from the million-row experiment.
It creates a disposable add-in copy and opens one workbook on a private desktop.
The original fixture remains unchanged. Only the resident's working copy is
saved; completed saves are copied for XML inspection before Excel closes.

## Method

Seven cases each have before/after sheets in the same workbook. Each sheet has
32 logical rows, numeric content in A except for empty row 8, and content below
the test row. The removed empty cell is B8 with a 30-point font. In the column
cases, entirely empty B inherits the same font instead. The explicit fixed
height is 40 points. No test changes column widths or calls AutoFit.

Office.js reads `Range.top`, `Range.height`, `Range.rowHidden`,
`Range.format.rowHeight`, and the next row's `top` for every sheet at three
stages: opening, ordinary `Workbook.save`, and full rebuild followed by save.
The verifier compares every geometry field across stages. For fixed cases it
also compares before/after geometry and saved height/hidden attributes.

The initial verification incorrectly equated the nominal 40-point setting with
`Range.height`, then incorrectly expected Save to retain the literal XML value
40. The retained measurements exposed both assumptions. Verification now checks
the actual intended invariants: paired geometry and height attributes remain
equal, while each distinct API measurement stays unchanged across stages.

## Results on 2026-11-01

Windows Excel **16.0.20527.20088**. The outcome was identical at all three stages:

| Case | Before removal | After removal | Outcome |
| --- | --- | --- | --- |
| Fixed height, remove empty font cell | Nominal 40 pt | Nominal 40 pt | All measured geometry unchanged |
| Fixed heights, remove empty column font | Nominal 40 pt | Nominal 40 pt | All measured geometry unchanged |
| Automatic height, remove empty font cell | 39 pt | 15 pt | Height and next-row position change |
| Automatic height, remove empty column font | 39 pt | 15 pt | Heights and downstream positions change |
| Stored `ht="40"`, but `customHeight="0"` | 39 pt | 15 pt | Stored `ht` alone does not fix height |
| Delete the entire fixed-height row | Nominal 40 pt | Default 15 pt | Lost fixed height is correctly detected |
| Fixed hidden row, remove empty font cell | Hidden, zero extent | Hidden, zero extent | Hidden geometry unchanged |

The seven pairs were measured together in **34.9 ms** after opening. Save plus
measurement took **102.5 ms**; Full Rebuild, save and measurement took **100 ms**.
A repeat passed the same checks, taking 28.1, 76.2 and 74.6 ms respectively.
These small-sheet timings are not million-row benchmarks. The earlier
[million-row experiment](../officejs-million-row-heights/README.md) measures that
separate question. The owned Excel process exited and the fixture hash remained
unchanged. The [compact results](results/2026-11-01.json) retain all geometry.

### Distinct representations of a fixed height

For the same nominal 40-point fixed row, Office.js reported
`format.rowHeight = 40` and `Range.height = 39.9375`. Its `top` and the next
row's `top` were independently reported; their difference was 39.75 in the
single-row case. These API values are not interchangeable.

Ordinary Save rewrote `ht="40"` to `ht="39.950000000000003"` on both sides,
retaining `customHeight="1"`. Full Rebuild plus Save retained that same value.
The open-workbook geometry did not change. This test observes those numerical
differences; it does not establish their internal rounding cause or test
reopening the saved files.

## Implication for candidate selection

The results support comparing **retained** explicit fixed heights in XML,
instead of separately measuring every such row when only empty font sources
were removed. Require a matching row, explicit fixed-height status on both
sides, equal height and unchanged hidden state. Missing rows, changed height
attributes or changed fixed/automatic status remain candidates. A present `ht`
without fixed-height status is insufficient.

This only excludes a possible **height change**. Changed font rendering, fills,
borders, objects or values still require their own candidate analysis. It also
does not establish unchanged absolute positions: automatic-height changes in
earlier rows can shift a fixed-height row. The shared XML/geometry planner must
account for those displacements separately.

The earlier EUSES case removed the entire trailing fixed-height row, including
its height definition. It does not satisfy the retained-row condition and must
not be skipped. The targeted comparison has not yet been changed to apply this
policy. These are controlled synthetic cases, not a guarantee for arbitrary
Excel edits, AutoFit operations or all workbook features.
