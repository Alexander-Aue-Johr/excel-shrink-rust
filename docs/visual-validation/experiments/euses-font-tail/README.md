# EUSES: empty font overrides and row geometry

Investigation on 2026-11-01, source commit `3efd3a5`. No production shrinker,
capture scheduler, or running corpus was changed. All Excel probes used
disposable copies on a private desktop, without recalculation or saving.

## Main result

The supplied `imp_emp_fin_bo_report_A8C19.xls--23d7c3ac` case already shrinks
almost equally well with Default and Maximum in the current source:

| Input or profile | Bytes | Cell records |
| --- | ---: | ---: |
| Excel-resaved comparison input | 602,365 | 130,984 |
| Default supplied on USB | 491,807 | 130,984 |
| Fresh Default | 17,790 | 198 |
| Fresh Maximum | 17,789 | 193 |

The fresh CLI was built offline from the existing source. Its SHA-256 was
`a189d6ed042078dc3d96415be1c3157f4333f2a61407b38daacd668520d085f9`.
The live release binary and corpus process were left alone.

The original populated region and sampled empty-tail regions render identically
after fresh Default. However, controlled variants expose a real correctness gap
in the existing trailing-layout-row cleanup: it can remove font-dependent or
explicit row heights while inherited column borders remain visible.

## Why the old result retained so much

The last nonempty cell is `Sheet1!C52`. Rows **53:65445** contain **65,393**
empty A/B pairs: **130,786 empty cell records**. The uncompressed sheet XML is
6,387,551 bytes. Fresh Default reduces it to 11,860 bytes.

These cells pass a separate **column-font override** guard. They are not merely
one generic font sentinel per row. Both A and B use font 2 while their column
styles use font 0:

```xml
<!-- Cell font 2 -->
<font><sz val="10"/><name val="Arial"/><family val="2"/></font>

<!-- Inherited column font 0 -->
<font><sz val="10"/><name val="Arial"/></font>
```

The fonts are not byte-identical, so the existing font-identity cache keeps them
distinct. Both cells can survive independently of the ordinary per-row font
representative set. B also has explicit `wrapText="1"`, unlike its column
style; these cells contain no text to wrap.

The cell borders are also present in the column styles. Removing the cells
therefore does not remove those borders. That helps explain the good real-case
result, but makes a changed row height visible in the counterexamples.

There are no merges, conditional-formatting rules, or drawings on this sheet.
The empty-tail rows have no explicit `ht` or `customHeight` attributes.

## What Excel actually showed

For the real input, Default, and Maximum:

- `A53:A65445` has uniform measured row height **12.75 pt**, top **4053.75 pt**,
  and total height **833760.75 pt** in all three files.
- Images of `A1:F52`, `A45:F62`, `A995:F1008`, and `A65440:F65450` have equal
  dimensions and exactly equal decoded RGBA pixels.
- Nonempty values/formulas are unchanged in this case.

This checks the complete populated region, three tail samples, and a uniform
height query over the entire empty tail. It is not a universal proof for every
font, Excel version, workbook dependency, or editing operation.

Controlled copies of the same workbook separate the relevant conditions:

| Variant | Current Default result |
| --- | --- |
| Only `A1000` changed to 30 pt | Row 1000 changes from **37.5 to 12.75 pt**; its capture differs. |
| All empty A/B tail fonts changed to 30 pt | Tail changes from **37.5 to 12.75 pt**; all three sampled images differ. |
| Same 30 pt tail, with content added at `C65446` | Tail remains 37.5 pt; sampled images match. |
| Same anchored tail, retain only A's font sentinel per row | Remains 37.5 pt; images also match the two-sentinel control. |
| Same anchored tail, remove both sentinels | Changes to 12.75 pt; images differ from the two-sentinel control. |
| 30 pt tail and explicit 40 pt custom row heights | Current Default removes the tail; heights become 12.75 pt and images differ. |
| Original 10 pt fonts, only add explicit 40 pt custom row heights | Same regression; the font-size change is not required. |
| 30 pt tail with columns A/B hidden | Heights still change from 37.5 to 12.75 pt; visible regions differ. |
| 30 pt tail with blue cell fill | The painted cells remain; heights and sampled images match. |

The hidden-column case rules out treating all fonts in hidden columns as
irrelevant to row geometry. The content-below case also shows why a trailing
empty area and an empty area between populated rows need different treatment.

Attribution check on the single-cell variant:

- `--disable-tail-pruning` alone still removes the changed font cell and tail.
- `--disable-trailing-layout-only-rows` preserves them.

The unsafe removal happens in the later layout-only row cutoff. It is not
explained solely by the initial tail-range pruning pass.

## A promising improvement for the remaining large gap

Fresh Default on `Electricity_2520Meters_25_A866E.xls--99680b36` is 249,567 bytes;
Maximum is 73,639 bytes. Much of the difference is in two populated sheets:

- `Exports Imports Units`: 13,105 empty cells among 384 retained rows.
- `Exports Iports $`: 50,250 empty cells among 271 retained rows.

Many empty cells repeat the same Arial 8 pt font 7. Their inherited column
font is usually Arial 8 pt font 2, again differing by the family metadata.
Styles 29 and 49 have no border, fill, or alignment children; their number
formats differ. Other styles and workbook parts were left untouched.

`make_sentinel_candidate.py` is a fixture-specific experiment, **not a general
cleanup implementation**. Within those two sheets it removes redundant empty
cells with styles 29/49 after ensuring another font-7 cell remains in that row.
An existing content cell or another retained font-7 style can serve as that
representative. It never moves a representative or removes a nonempty cell.

Results:

- **60,646 empty cells removed**; workbook cell count falls from 69,554 to 8,908.
- Candidate size: **86,647 bytes**, versus Maximum's 73,639 bytes.
- Python ZIP level 6 was used for the candidate. Repacking the unchanged
  Default with the same writer gives **216,898 bytes**. Thus compression/writer
  differences account for part of the 249,567-to-86,647 difference; deleting
  cells accounts for the 216,898-to-86,647 difference under the same writer.
- All **655** retained row heights and positions match between the unshrunk
  comparison input, fresh Default, and candidate.
- Ten sampled image regions match exactly across those three files.
- Only sheet9.xml and sheet10.xml changed; their nonempty cell subtrees did not.

The real-case result supports deduplicating repeated font protection inside a
row. It does not prove that any arbitrary same-font cell is a sufficient
representative in merged, hidden, wrapped, or otherwise complex layouts.

## Recommended implementation order

1. **Correct the layout-only tail cutoff first.** Preserved column borders/fills,
   explicit row heights, and a cell's font contribution must be considered
   together. A cell may add no new paint but still affect where inherited paint
   is drawn. Preserve unresolved geometry rather than assuming a textless row
   has no visual effect.
2. **Deduplicate column-font protection per row in a narrow supported case.**
   Initially limit this to plain empty, unmerged cells and verified compatible
   inheritance/layout. Keep representatives at their original positions. Use
   only representatives that survive every later cleanup pass. Painted cells,
   protection, and other independently required styles remain at their own
   positions. Reuse existing row state and font identities instead of adding a
   second sheet-wide analysis pass.
3. **Do not globally equate fonts by name and point size.** Family, charset,
   scheme/theme, bold/italic, superscript/subscript, and installed font fallback
   deserve separate consideration. The measured Arial cases do not establish
   that omitted family metadata is always irrelevant on every machine.
4. **Expand only with concrete counterexamples.** Mixed fonts in one row,
   different effective column/row styles, hidden rows/columns, merged regions,
   wrapping, rotation, fixed/automatic heights, later content, drawings, and
   conditional formatting are relevant boundaries. Style-inspecting formulas
   are another reason not to treat every empty-cell style as interchangeable.

Promoting A/B's style to the whole row is not a general shortcut: it changes
implicit cells in other columns. Setting saved row heights is also a separate
behavioral change and can turn automatic layout into fixed layout. Neither
approach was adopted.

There is no size benefit worth pursuing between 17,790 and 17,789 bytes in the
main case. The useful work is fixing the demonstrated geometry regression and
then reducing the repeated font guards in cases such as Electricity.

## Source guide

- [Font identities](../../../../excel-shrink-core/src/styles.rs):
  `parse_font_identities` reuses byte-identical font records.
- [Cell retention](../../../../excel-shrink-core/src/sheet/cells.rs):
  `empty_cell_has_column_font_style_override`, `keep_column_font_override`, and
  `kept_cell_is_tail_anchor` explain the independent cell and tail decisions.
- [Row representatives](../../../../excel-shrink-core/src/sheet/rows.rs):
  `select_empty_style_representative` implements the ordinary per-row policy.
- [Final tail cleanup](../../../../excel-shrink-core/src/sheet/output.rs):
  `tail_boundary_row`, `trailing_layout_only_rows_cutoff`, and
  `cutoff_after_row_boundary` determine the later removal.
- [Earlier row-top experiment](../row-top-change/README.md) documents another
  font-dependent row geometry change.

## Scope and reproduction

All 72 workbook files in the USB handoff were checked against its size/hash
manifest. All twelve supplied EUSES comparison inputs were processed with the
fresh Default and Maximum profiles. This is a selected diagnostic set, not a
representative sample of EUSES or the whole corpus.

The payload checks were not universally equal: Business_Issues loses 490
nonempty cells in hidden rows under the existing hidden-content policy; Maximum
also removes one whitespace-only cell in each of the two local quarterly
reports. Those changes are separate from the empty-font experiment. Eleven of
twelve Default payload checks and nine of twelve Maximum checks match.

`recorded-results.json` contains the measured size/geometry/image outcomes;
the local `.tmp/euses-font-study/report.html` additionally links the PNG evidence.
Workbooks and images are not committed.

Generate the nine controlled tail variants:

```powershell
python docs/visual-validation/experiments/euses-font-tail/make_variants.py `
  .tmp/euses-font-study/controls/23d7c3ac.xlsx .tmp/euses-font-study/variants
```

Shrink each variant with the current CLI. For Maximum, additionally pass:

```text
--remove-textless-default-font-representatives
--remove-textless-style-representatives
--remove-textless-column-style-representatives
--remove-unreferenced-whitespace-cells
```

Create the separate Electricity candidate from fresh Default:

```powershell
python docs/visual-validation/experiments/euses-font-tail/make_sentinel_candidate.py `
  .tmp/euses-font-study/default/99680b36.xlsx .tmp/euses-font-study/electricity-one-sentinel.xlsx
```

Probe plans are JSON arrays. Each entry provides `name`, absolute workbook
`path`, and a `ranges` list. Optional `sheet`, `rows`, and `tail` properties
select another sheet and row-geometry queries. Defaults target the main
EUSES tail. Each workbook must be closed before its next probe.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  docs/visual-validation/experiments/euses-font-tail/run_probe.ps1 `
  -Output .tmp/euses-font-study/new-probe -Plan .tmp/euses-font-study/plan.json
```

The runner reuses the existing Check Performance experiment's owned-process,
private-desktop, disposable-worker, and Office.js host helpers. It requires
Windows, desktop Excel, Python, and the existing release CLI. Image comparison
uses Pillow. The summary generator expects the four named probe output folders
used by this investigation; it fails on missing or mismatched probe data.

Verification performed: offline debug build; all 54 existing core tests matching
`font`; 27 isolated workbook probes; exact pixel and row-geometry comparisons;
recreated variants checked against the tested workbook parts. Existing core
tests passed but do not cover the demonstrated inherited-border tail regressions.
