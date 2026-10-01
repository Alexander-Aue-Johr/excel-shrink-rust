# Real-world visual regression cases

These cases are kept as research notes for future regression fixtures, paper figures,
and presentation slides.

## Hidden wrapped cell changes a visible automatic row height

Workbook: `statistischer-bericht-rechnungsergebnis-kernhaushalt-gemeinden-2140331217005.xlsx`

Observed case:
- worksheet `71717-50` (sheet 55 in the investigated workbook)
- hidden column `A`
- cell `A4`, text `Spalten-/ Zeilen-Nr.`
- wrapped-text style
- row 4 has `ht="25.5"` but no `customHeight="1"`
- workbook default row height is `12.75`

Removing the unreferenced hidden-content cell makes Excel recompute the automatic row
height. The screenshot becomes 17 pixels shorter and the visible suffix moves up.
The visual detector classified it as pure vertical suffix reflow: restoring 17 pixels
of spacer height reconstructed the original with zero residual differing pixels.
The workbook metadata is consistent with that observation at 96 DPI:
`25.5 pt - 12.75 pt = 12.75 pt = 17 px`.

Why it matters: hidden and unreferenced content can still influence visible layout.
For wrapped text, the rendered line count depends on the value, column width, font
metrics, indentation, and Excel layout behavior.

Planned regression — separate commit: add a dedicated real-workbook case for this
workbook/sheet/cell and preserve the observed row-height behavior.

Planned production fix — separate commit, intentionally NOT in the validation patch:
keep hidden-content candidates when their style can affect an automatically sized row.
Treat `wrapText=true` conservatively when the row is not fixed by `customHeight="1"`.
Evaluate rotation, unusually large font metrics, and rich text separately.

## Unicode sheet names exposed a screenshot-cache boundary bug

The same workbook exposed false cache failures for `Inhaltsübersicht` and
`Erläuterung zu CSV-Tabellen`. Rust writes cache-plan JSON as UTF-8 without a BOM;
Windows PowerShell 5.1 therefore reads this interface explicitly with `-Encoding UTF8`.
This is a validation-infrastructure bug, not a cleaning-rule regression.
