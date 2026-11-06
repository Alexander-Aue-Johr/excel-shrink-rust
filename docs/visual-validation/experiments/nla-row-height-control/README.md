# NLA automatic-height control

## Question

The launcher also accepts `-Sheet 'Worksheet name'` for focused controls on other
workbooks. Its default remains `2007 NLA QA Data`. The measurement range remains
rows 6–179, so this option changes the selected worksheet without implying full
height coverage of another workbook.

The Data.gov comparison of `NLA water isotope data-2007-2017_5289b0294534.xlsx`
reported heights of 15.75 versus 15 points for rows 6–179 of
`2007 NLA QA Data`. Both states were rebuilt in their resident processes.
The resulting one-pixel-per-row displacement produces a large image difference.

## Corrected worksheet mapping

The affected worksheet is `xl/worksheets/sheet5.xml`. The initial experiments
incorrectly treated it as `sheet2.xml`, which belongs to
`NLA_07_water_isotopes`. Restoring that other worksheet could not establish
that the affected worksheet was unchanged. The earlier interpretation has been
withdrawn. Native commands used the worksheet name throughout.

## Counterfactuals observed on 2026-11-06

Experiments used disposable copies, private desktops and runner build
`77bbca5`. One regular group slot was reserved for each native pair. Corpus
bundles and receipts were preserved.

| Pair or isolated change | Original height | Other height |
| --- | --- | --- |
| Two byte-identical original inputs | 15.75 pt | 15.75 pt |
| Original parts repacked with Default ZIP entry order and metadata | 15.75 pt | 15.75 pt |
| Only `sheet6.xml` or only `sheet7.xml` replaced with shrunk XML | 15.75 pt | 15.75 pt |
| Only `sheet5.xml` replaced with shrunk XML | 15.75 pt | 15 pt |
| Only the column definitions of `sheet5.xml` changed | 15.75 pt | 15 pt |
| Only the cell removals and span updates of `sheet5.xml` changed | 15.75 pt | 15.75 pt |
| Full Default with one original column retained at AS | 15.75 pt | 15.75 pt |

Each result held directly after opening, after saving and after full rebuild
and save. Both owned Excel processes confirmed closure after every successful
probe. A malformed intermediate ZIP fixture failed the inventory guard before
workbooks were opened. Its replacement fixtures passed CRC and single-part
difference checks before use.

## Cause and minimal correction

The first 44 columns use Arial 10. Original AS has no explicit style and AT is
an uncovered gap, so both retain the default Calibri 11 font. The later column
tail uses Arial 10 again. Collapsing AS through XFD into the final tail style
removes the last default-font columns. Those empty columns nevertheless
determine automatic heights throughout the data rows.

Keeping AS as one default-font column and merging AT through XFD preserves the
observed heights. No per-row height assignments or additional data cells are
needed. This confirms a shrink-induced layout change, rather than independent
session variation in these controls.

## Reproduction

`run.ps1` starts `same_package.py` on an owned private desktop. Supply Python,
the CLI binary, an original XLSX and a new output directory. Omit `-Default`
for the identical-input control. Supply a disposable counterfactual with
`-Default` for the other controls. The script uses the production host and its
Office.js height, geometry, save, rebuild and close commands. Every request is
bounded and the outer cleanup job owns all experiment processes.

Local evidence is retained below
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp` in
`nla-same-package-proof`, `nla-zip-layout-proof`,
`nla-isolate-sheet5-proof`, `nla-isolate-sheet6-verified-proof`,
`nla-isolate-sheet7-verified-proof`, `nla-sheet5-columns-only-proof`,
`nla-sheet5-cells-only-proof` and `nla-column-font-sentinel-proof`.

## Production verification and cache migration

Core fix `44e9537` retains canonical column-font sentinels. The complete core
suite passed with 266 unit tests and four allocation tests, with one ignored
test. A fresh CLI shrink produced a 662,460-byte Default file, compared with
662,446 bytes before the fix and 662,562 original bytes.

The production targeted workflow compared seven regions in each rebuilt state.
All seven image pairs were captured and compared without pixel, geometry or
rendering differences. The effective-height check for the affected worksheet
also found no differences. Full evidence is retained in
`validation-temp/nla-production-font-fix-proof/result` beside the other probes.

A read-only inventory examined all 8,623 prepared bundles. It checked column
headers and format identities without scanning worksheet cells, excluding
already measured outputs with no column merge. Nineteen bundles had changed
column fonts or gap fills. Replaying those nineteen with the new core produced
changed package parts in every case, with no inventory or replay errors.

Their old bundles and records were copied and SHA-256 verified before targeted
invalidation. Reports and PNGs remain retained. The backup is
`validation-temp/backups/before-column-font-20261106-020535`.
The full run resumed with build SHA-256
`c201e1668d0baad9d4707e4e40852a0da6c20170379fb17bca30fecdaed55224`.
All nineteen replacement bundles were verified as prepared by that build.
Their production visual comparisons remain work for the resumed run.
