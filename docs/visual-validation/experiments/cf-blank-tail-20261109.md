# Conditional formatting on unserialized blank tails

## Question and native reference

Can a supported `cellIs equal` rule with a nonempty literal text operand
contribute visible formatting beyond every serialized cell?

Four small worksheets use operands `"."`, `"0"`, `""` and numeric `0`.
Each contains a dot at A1, numeric zero at A2, text zero at A3, an empty-string
formula at A4 and an explicit blank A5. The differential format is an opaque red
solid fill. The remaining cells have no serialized records. One workbook applies
the rules to `A1:B1048576`, and its partner applies them to `A1:B5`.

The independent native reference command rebuilt both workbooks and captured
exactly `A1:B20` on each sheet. Native object inventories were empty. The
reference completed in 10.784792 seconds. Both images were 169 × 401 pixels in
each pair. Exact overlapping RGBA comparisons gave the following observations.

| Operand | Changed pixels | Observed blank tail |
| --- | --- | --- |
| `"."` | 0 | No contribution |
| `"0"` | 0 | No contribution |
| `""` | 50,400 | Red fill below row 5 |
| `0` | 50,400 | Red fill below row 5 |

The numerical-zero and empty-text controls prevent treating every constant
operand as inactive on blanks. A numeric value and a quoted number remain
different operand types.

A second four-sheet reference checked the literal operands `"*"`, `"?"`,
`" "` and `"~*"` with the same full/finite application limits. It completed
in 12.3236682 seconds and found zero changed pixels in all four pairs. This
countercheck guards against mistakenly treating these `cellIs equal` operands
as wildcard predicates matching empty cells.

## Production-selector controls

Two positive-only sheets were compared through the ordinary targeted launcher.
Changing only their CF application limits produced two selected regions,
`A1:B5` on each sheet, with no changed pixels. The attempt completed in
10.936 seconds and recorded `cf_blank_tail_excluded=true` for both sheets.

A separate partner added a dot at A20 on the first sheet. The union envelope
expanded through row 21, and the ordinary comparison detected the changed first
sheet while the second remained equal. This attempt completed in 10.786 seconds.
Thus the extent rule does not discard newly serialized values beyond the
previous data boundary. Both attempts retain normal limited-coverage status.

## Retained Destatis check and limits

An XML-only audit of all 27 sheets in the retained Baumschulerhebung original
completed in 0.91 seconds. Twenty-six visual extents became finite. The
`Vorbemerkung` sheet retained the whole-grid fallback because it contains legacy
drawing/OLE markup. This workbook is therefore still unresolved as a whole.
No corpus verdict was replaced by these diagnostic observations.

The proof requires supported namespaces, literal grammar and rule structure,
resolved column metadata and supported worksheet features. Numeric and empty
text operands, other operators, dynamic expressions, extensions and unknown
objects retain the existing fallback. It concerns implicit blank tails only.
Actual cell inputs, their formulas and rule changes retain ordinary selection.
These controls do not establish corpus-wide recall or a throughput estimate.

## Evidence identities

Native inputs, plans, PNGs, rebuilt copies, reports and launcher receipts remain
under `G:\deltasieve-work\cf-blank-tail-20261109`.

| Input | SHA-256 |
| --- | --- |
| Four-sheet full application | `fe4b3ebf043f272e37abd84e4c106fddb6ccb28e07833f89ca7e8de0ff49f992` |
| Four-sheet finite application | `ac65813f6d127ad3e41f24bb003b4c823468570b520676fe5fbf93377c961416` |
| Positive full application | `8b29459ad4e14e7b725e70cc894df1661c2253fa7b6aa02427b187768695360d` |
| Positive finite application | `426744f45b97948fec4e1eb769ba5fd00fb16fb84a4dea624c7396b7384c08ed` |
| Added A20 dot | `f474fe89d5aab5a413a13650d302982982746e62141c9b6ffd89d4c543532db1` |

The independent reference used the previously packaged `0ad30f53` runtime and
bypassed production candidate selection. The ordinary controls used the newly
built diagnostic executable and matching current assets. Their bytes and
operation evidence are retained separately from the running corpus runtime.

## Optimized package check

The separately packaged optimized runtime passed the ordinary positive control
in 11.108 seconds, selecting two regions with no pixel differences and retaining
both CF extent proof markers. All 114 runtime members were verified. Its
application SHA-256 is
`864f251586b6598608090286cbb8f263bd4bc580ac3bf7c7bb5235ecb96c96dc`,
and the native report hash is
`fcd8884409626b34ac47a3e0e3ae64be83b25b660ff69deb2975f6cb6182727c`.
The package is retained in the private night-work directory as
`runtime-2d72e076`. This is package acceptance, not production deployment or
corpus completion.
