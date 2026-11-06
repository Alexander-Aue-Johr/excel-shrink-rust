# Implicit default row formatting

## Observed case

`2020_01_08_AFFF_Manual_3299b4bdfc0f.xlsx`, worksheet `Data`, lost portions of
its blue background in the rebuilt Default capture. The affected area includes
G:M of rows 124–128. Row 124 is empty, which rules out a text-overflow-only
explanation.

The original row headers contain `customFormat="1"` without an `s` attribute.
Their concrete colored cells restore the blue fill over the implicit row XF 0.
The previous parser treated an omitted `s` as absence of any row formatting,
then deleted colored cells whose fill matched their column.

## Native controls

Controls used separate XLSX copies on owned private desktops, Check Performance
disabled and the production paired rebuild comparison. The originals and corpus
receipts remained intact. Runner `44e9537` provided the baseline behavior.

| Change to the previously shrunk copy | Result |
| --- | --- |
| None | Background differs in Data rows 124–128 |
| Restore complete original rows 125–129 | Only the row-124 difference remains |
| Restore complete original rows 124–129 | No image difference |
| Restore only original G:M cells in rows 124–128 | No image difference |
| Remove only `customFormat="1"` from rows 124–128 | No image difference |

The final control isolates the row-header semantics. Removing those flags is a
diagnostic counterfactual. Production cleanup must retain the original row
format and the concrete cells needed to preserve its appearance.

## Implementation boundary

`RowView::parse` resolves XF 0 when `customFormat` is true and `s` is omitted.
Existing cell-versus-row-versus-column comparisons then retain the necessary
cells. Explicit style indices keep their existing behavior. Ordinary rows and
`customFormat="0"` continue to inherit column formatting.

The focused core test covers both true encodings, the absent/false cases, an
explicit nonzero row style and preservation of a concrete colored cell. The
temporary overflow-span implementation was withdrawn after the empty-row
control exposed the more specific cause.

## Production verification

On 2026-11-06 the corrected Default output completed the same native paired
rebuild test with one selected Data region and zero pixel or geometry
differences. Both Excel processes confirmed closure. The original package was
234,671 bytes, the old Default 231,939 bytes and the corrected Default 232,255
bytes. The fix adds 316 bytes to retain the required formatting.

All 267 active core unit tests and four allocation tests passed. One existing
unit test remained ignored. The broad corpus continues independently while
the affected prepared-cache inventory is examined.

## Prepared-cache audit and deployment

The complete inventory examined 8,623 prepared originals and found 155 workbooks
with an implicit default row format. Replaying all 155 changed 20 Default
packages. The other 135 kept identical package-part contents. One 62 MB input
exceeded the initial five-minute audit deadline; its longer retry completed and
confirmed unchanged part contents. No unresolved inventory/replay errors remain.

Only the 20 changed prepared bundles were backed up and invalidated. Their old
records, reports and images remain retained. Runner build `499a64f`, SHA-256
`09e2353aec93cae2de4fe819b659e1139e6bb8e416836a49fdd7e1e0c0e2faed`,
resumed the same study with four groups and regenerated those outputs. This is
an artifact compatibility audit, separate from completion of their native visual
comparisons and the remaining corpus.
