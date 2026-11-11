# Verkehrsunfälle coverage and PFAS Inquire observations

## Verkehrsunfälle and SheetShrink

The bounded whole-workbook attempt remains incomplete. It retained rebuilt
workbooks and capture-selection inputs before the final capture plan.
The original rebuild has SHA-256
`9a8d6239fb2394fb096deb571958286874a957ca40ed8dc4dc9cb9eb7ce031aa`.
The SheetShrink rebuild has SHA-256
`f7ea0a3e3573c20144b09a23091db725bbae4674fd6b272992ad4597329b583c`.

Archive inspection of sheet `10.1` found these row representations:

| Observation | Original rebuild | SheetShrink rebuild |
| --- | ---: | ---: |
| Last serialized row | 1,048,576 | 88 |
| Explicit hidden rows after row 90 | 1,048,486 | 0 |
| Default zero height | 1 | 1 |
| Two-cell drawing anchors containing shapes | 9 | 9 |

Original rows 89 and 90 have explicit heights of 9.95 and 8.25 points.
Row 89 contains eight cell records without value, formula, or inline string
payloads. Row 90 contains no cell records. Both rows are absent from the
SheetShrink rebuild. The inherited column style spans all columns in each
rebuild and specifies fill 5 and border 0. Neither sheet contains conditional
formatting blocks.

The explicit hidden tail accounts for the full-height XML dimension on this
sheet. The native visibility and ink extent of the shapes and the two
explicit-height rows still need evidence before narrowing capture coverage.
Sheet `1.3` illustrates a separate boundary. Both XML dimensions end at
`AA95`, yet the saved selection envelope covers the entire worksheet.
Each rebuild has four two-cell shape anchors and inherited fills reaching
column XFD.

## PFAS and Inquire

The `PFAS_BSAF_DATA__2__49658e04b555.xlsx` comparison completed with limited
coverage. The packaged report has SHA-256
`091cab656b5bb08744e212ba008218bf09be7a91f63aed734c6ff343d22412a8`.
It recorded 4,277 regions and 129 pixel findings. Of these, 128 are on
`BSAF_Data (2)` and one is on `References`. Among the 128 findings on
`BSAF_Data (2)`, 126 also have a 24-point horizontal origin difference.
There are 127 regions with exactly 24,579 changed common pixels.

In `BSAF_Data (2)!ACO1:AHL84`, both images are 8,193 by 1,727 pixels.
At image coordinate x=1, rows 64 through 66 are RGB (112, 48, 160) in the
original and RGB (255, 255, 255) after Inquire. The adjacent rows 63 and 67
are white in both. The report records 24,579 changed common pixels and zero
unmatched nonwhite pixels for this region. This establishes a change inside
the retained image pair. The exact formatting change responsible for that
stripe remains unverified.

Capture selection and transport took 1,668,137 ms. Image comparison took
7,305 ms. These phase durations describe the retained run, whose producer
used capture policy v15.

## Stage-nine timing snapshot

A snapshot of 68 completed external-tool pair reports across the three
corpora on 2026-11-10 includes only reports with the
`capture_selection_and_transport` timing. It excludes SheetShrink, failed
and pending pairs, and reports without that timing. Totals sum pair
durations and can overlap in wall-clock time.

| Candidate producer | Timed pairs | Selection and transport (s) | Image comparison (s) |
|---|---:|---:|---:|
| Check Performance | 22 | 740.0 | 24.1 |
| Inquire | 14 | 5,322.9 | 446.7 |
| NXPowerLite | 12 | 878.7 | 21.0 |
| XLStylesTool | 20 | 14.7 | 1.8 |

The longest measured selection and transport phases were 27.8 min for PFAS,
21.6 min for Hospital Data, and 17.1 min for Enron pair
`a90ce85ed998ecc4a267c5e0`, all with Inquire candidates. They each produced
at least 2,975 worksheet images per side. Hospital Data also spent 4.3 min
in image comparison. These observations identify an upper tail to account
for before expanding the cohort. They do not isolate selection from
rendering and transport, measure the candidate-producing tools themselves,
or compare matched populations across tools.

### PFAS across three candidate producers

The three completed pair reports for `195786a2d0056e298e98a7ce` have
identical original package-member inventories, including SHA-256 values.
The candidate and selected regions vary by producer.

| Candidate producer | Worksheet captures per side | Differing regions | Selection and transport (s) |
|---|---:|---:|---:|
| Inquire | 4,277 | 129 | 1,668.137 |
| Check Performance | 63 | 0 | 24.025 |
| XLStylesTool | 0 | 0 | 0.032 |

All three are `completed_with_limitations`. XLStylesTool produced no
worksheet or chart captures for this pair. Check Performance measured 63
regions on `BSAF_Data (2)`. The zero counts therefore have different
coverage. A matched-region reference capture of all variants would be
needed to compare their rendering at the Inquire finding. This observation
does not support a tool-wide preservation claim or a comparison of the
tools' own cleanup speed.

Pair report SHA-256 values:

- Inquire `091cab656b5bb08744e212ba008218bf09be7a91f63aed734c6ff343d22412a8`
- Check Performance `7edbb8425f435055e6a67ca49391cfedab766f4ac73184b5da4bb7624349d0af`
- XLStylesTool `40526ab3e464f64042736c9e081db71e3f13d29f57d36896397ae984836c3929`

### QT/RR expands a narrow used range to full width

Data.gov pair `4f6a5ec3f215137b2ef2a5c2` (QT/RR dobutamine challenge)
was still running when inspected. Inquire's v15 plan shows four inputs
per affected sheet across 11 sheets and 90,112 measured capture regions.
All 11 input envelopes reach column XFD. Both rebuilt packages declare
dimensions ending at L on those sheets, with 3,152 to 10,586 rows. Their
twelfth sheet declares A1. There are no explicit column records in the
worksheet headers. The rebuilt original leaves the default column width
implicit. The Inquire candidate sets `defaultColWidth="9.140625"` and
`customHeight="1"`, with `defaultRowHeight="12.75"` on both sides.
On the inspected first rows, the original also uses a row style while the
candidate writes some explicitly styled empty cells. The widths and row
styles can affect visual output beyond the cells with values.

The reported dimensions therefore cannot justify clipping the comparison
to column L. A bounded reference check should measure effective column
widths and probe the inherited row background before designing a narrower
plan. A running counter recorded 2,238 successful individual captures
about 12 minutes after pair start. Extrapolation to both sides is provisional
because regions vary in cost. No equality or full-coverage conclusion
follows from this snapshot.

Rebuilt archive SHA-256 values:

- Original `b43aee196cd16c562d85a6b88bf0278f31bb0b7e178af88194b0d14e7a28b8e4`
- Inquire `1607f68b64841e025660c7b95dfd0c99264ff16853c8bd515f789e87a984d25f`

The full rebuilt worksheet XML confirms exactly eleven explicit row
styles in the original: style 2 on row 1 of each affected sheet. The
Inquire candidate has no explicit row styles. Style 2 resolves to a thin
bottom border (border 1, indexed color 64) and no fill in both style
tables. A full-width inherited border is therefore a concrete candidate
for a visual difference outside column L. The earlier suggestion to
probe the row background should include this border explicitly.
A bounded reference can capture row 1 beyond L and sample it across the
tail, independently of the default-width measurement. Any future
analytical pruning must preserve this signal and distinguish the two
dependencies.

XLStylesTool completed the same workbook with zero selected worksheet
regions and zero chart captures under limited coverage. Its terminal
report SHA-256 is
`f12cc28a57dc2a5cbee348b5314d20164f1ce28b5563561a98efa667ab8eb079`.
The 78 ms recorded selection-and-transport phase applies to that
zero-capture observation.

## Raspberries XLStylesTool opening exception

Pair `11bfcc0d562a76ea178fe7fc` failed in the candidate opening lane.
Excel displayed a content-recovery prompt. The launcher recorded
`repair_required` and confirmed shutdown without accepting the prompt.
The Check Performance comparison of this workbook completed with
limited coverage in its separate run.

Both input ZIPs passed CRC verification. Their member lists match.
Only `xl/styles.xml` and `xl/workbook.xml` differ by member size or CRC.
Both XML parts parse, and the counts of style-table entries match.
The candidate contains 100 defined names versus 103 in the original.
Comparing name, scope and formula tuples found three removals with
no surviving definition of the same name in the same scope. These
archive observations do not identify which change triggered Excel's
repair prompt. The raw candidate remains an opening exception.

- Original SHA-256 `96fd0e60120c3878da261862433e2535f308f26299704ecc5a68b6958caf4ed8`
- Candidate SHA-256 `ed777965d9e8d793835d6e7cab19245b66eec7cff0f581e1b1a8a3c7dac1d385`
- Dialog receipt SHA-256 `06b9aebe1d3bea7e1f32f841ea8bcc90bdcb850161849699102f4dd97f90b2b9`

## Hospital Data capture cost by candidate

Four completed comparisons of Data.gov pair `206ec7d9d777560e7dbb7626`
have identical original package-member SHA-256 inventories. They all
reported zero differing regions under limited coverage.

| Candidate producer | Worksheet captures per side | Selection and transport (s) | Image comparison (s) |
|---|---:|---:|---:|
| Inquire | 3,553 | 1,296.010 | 260.367 |
| NXPowerLite | 4 | 1.921 | 0.293 |
| Check Performance | 4 | 1.389 | 0.295 |
| XLStylesTool | 0 | 0.087 | 0.007 |

NXPowerLite recorded 4,530,112 original-to-candidate input XML changes.
After rebuild and selection, 904 requested candidates on Financial and
Utilization Data produced eight grid tiles and four measured capture
regions. Inquire had 905 requested candidates on that same sheet and
recorded 3,297 measured regions there, plus 256 on Cost Allocation Data.
An early XML count and a pre-grid candidate count therefore cannot
predict the capture cost without accounting for candidate geometry.
The variants and selected scopes differ, so these measurements provide
neither a matched algorithm speedup nor a whole-workbook equality claim.

Pair report SHA-256 values:

- Inquire `8be72b8885bd1676665054b0f79ec05d6ed50de34f3556f494dcbbd8c25d9650`
- NXPowerLite `8b438a95ed2c84da3cc0ff563eaa1bd8f54ba3e6b277b3ee23b970ab4ebb6585`
- Check Performance `5896dbfd84a90fc91c531fc3b7f0721b4069fd12d89812c12c614aae5da0bc96`
- XLStylesTool `fd0a1de5e483933c5018341192e30136d4d52cbb57d3bc4e2674e8bbc5c1d916`

## Adage alternative nesting matched capture rectangles

For Data.gov pair `0d60ed9664980e4e59d37cd9`, the retained Inquire and
Check Performance captures share two exact worksheet/range keys. The
independently decoded original RGBA images were identical for each key.
The candidate image sizes differ, so the following measurements count
changed RGBA pixels in the top-left common image rectangle only. Both
reports have equal capture/content origins under their recorded policy.

| Range | Inquire changed common pixels | Check Performance changed common pixels |
|---|---:|---:|
| readme!A1:M20 | 76,444 | 83,105 |
| definitions!A1:G137 | 0 | 205,139 |

For definitions, the Inquire report records a blank-margin exclusion.
The other three observations record capture-extent exclusions and still
have changed common pixels. NXPowerLite retained 26 paired images with
zero reported differences, none at those exact range keys. XLStylesTool
selected no regions. These two matched rectangles can illustrate the
Inquire and Check Performance results together. A five-tool inset would
require new, matched reference captures for the remaining variants.

- Inquire report SHA-256 `435f1e66389fc978fc6c2a5f6af217c1d399e6f50125ada8fc4d8ae022a956eb`
- Check Performance report SHA-256 `a783acd72f0a82371fa260caf69ade8bdca30fb03cb5198b13f527ccd101b406`

### Adage fixed factor shares a rendered difference

On the distinct fixed-factor workbook, pair
`c569d1a453c80a90c0b8033b`, NXPowerLite, Inquire and Check
Performance all retained readme!A1:M22 at 1368 by 466 pixels on both
sides. Decoded RGBA SHA-256 values agree across runs within each
state. All three candidates change exactly 16,182 pixels in the
same bounding box (748, 20, 1306, 49), using zero-based coordinates
with exclusive upper bounds. The shared change is one rendered
outcome across three candidates. Its cause is unresolved. The
NXPowerLite input archives contain no formulas on readme, while the
report's dependency diagnostic exceeded its cell budget. XLStylesTool
has no readme capture in this run.

- Original RGBA `b0afcddb2547a02c3d4e51210f645083528959fa7ae65bba49c3847e5bc61f0b`
- Candidate RGBA `c200d295a3d87e41a6435e8f9873e213524b5aa1efe5918b27f87b27ad719a00`
- NXPowerLite report `fac8c6eae347ee2156be4a8345460e0ab6f9986fea41bd48df63da17b7c703dd`

Archive and image inspection explains the shared fixed-factor finding.
The original readme has fill 2 on A2:L2. The heading text in C2 is
unchanged in all three candidates. NXPowerLite, Inquire and Check
Performance each retain filled A2:C2 and omit the nine styled empty
cells D2:L2. Both captures have the same dimensions. Their visible
heading band ends at x=748 in the candidate and x=1306 in the
original, accounting for the complete 558 by 29 changed-pixel rectangle.

The current SheetShrink output (SHA-256
`34e3bd03cf03d3cfcac5c90ad901537c68bff25c79aa6a2b51fef854a287bf2a`)
retains fill 2 across A2:L2. The earlier identity-compatible visual
observation has 26 worksheet regions and no readme capture. The XML
preservation is directly observed. A matched SheetShrink readme capture
is still needed before using this as a measured four-tool illustration.
XLStylesTool also has no matching native capture.

Two bounded native reference runs completed the missing fixed-factor
captures. SheetShrink took 57.28 seconds and XLStylesTool 68.06 seconds.
Their original and candidate images all have dimensions 1368 by 466 and
decoded RGBA SHA-256
`b0afcddb2547a02c3d4e51210f645083528959fa7ae65bba49c3847e5bc61f0b`,
matching the original images from the three earlier runs. For this
post-hoc reference area, SheetShrink and XLStylesTool preserve the
heading band. Check Performance, Inquire and NXPowerLite shorten it.
The reference confirms this area only. Both existing comparison hosts
were still active after the bounded tests.

- SheetShrink reference receipt `523d824c815d14bdfab931e894e68e9a591485d7c915756d42ea1ae8f06dd43a`
- XLStylesTool reference receipt `24a0ea322bdd228773b73ae5e4c69f5e6945ad0c99060167adf1e2115f10cb89`

### Native QT/RR check beyond the used range

A 69.17-second explicit Inquire reference captured three ranges on
SHR1 DE. Original and candidate had identical geometry and 193 by 52
pixel images in each range. M1:O3 changed 192 pixels in a single
horizontal raster row. XFB1:XFD3 changed all 193 pixels of that row
from black to white. M20:O22 was pixel-identical.

This verifies that clipping at column L would miss a visible difference,
including at the far edge of the grid. The sampled blank-body range
and the three geometry measurements motivate separate treatment of
finite content and inherited borders. They do not establish equivalence
of the rest of the blank body or justify pruning the current plan.

Native receipt SHA-256
`f9a66dd439bedc52db9526d11b28233ad78a717b0b2be3b6b322c2a0f7531180`.

## Data.gov NXPowerLite observations under policy v15

PFAS completed 18 worksheet regions per side with no recorded visual
differences in those regions. QT/RR pair `4f6a5ec3f215137b2ef2a5c2`
completed with zero worksheet and chart captures. Neither result supplies
coverage of the separate Inquire findings at those workbooks.

LCCA pair `b7bddcbbcfce1e56d956df71` reported nine geometry differences.
In every case both widths were zero, both images absent, and heights
2000.25 and 1827 points. The five retained worksheet pairs and 27 chart
pairs had no recorded differences. These nine entries are nonvisual
geometry observations under the interpretation verified in v16, while
the original v15 verdicts remain intact. A native v16 run for this
NXPowerLite candidate has not been measured.

- PFAS report `f2860a56d712b5199ef75aa5d83d00bcd8208876b6978274d55c5b9246b8f98c`
- QT/RR report `e8dc4d011e283534e3fe1d78aa1e398e5d3fa0eb1a1cffc49c052fbf492a8b0c`
- LCCA report `9fba7cceb7c6e78521db3293657a4b77e30038a6476df02ad0b1de8ad250a1b6`

The live Inquire region index has 90,112 measured rectangles. Of these,
704 intersect columns A:L and 1,408 intersect row 1, with eleven in
both groups. Their union contains 2,101 rectangles. The other 88,011
intersect neither the declared finite content columns nor the verified
inherited border row. This is an accounting bound on a potential
layout-aware plan, not an accepted reduction. Before excluding the
other rectangles, the planner must account for all layout changes
(including effective default geometry), visible objects and inherited
appearance across both rebuilt workbooks. The running v15 plan remains
the reference for its recorded scope.

## Raspberries SheetShrink bounded reference

The ordinary SheetShrink comparison reported an unresolved full-grid
requirement on By Country. In the current raw input packages the sheet
dimension remains C5:AA53. Both inputs retain the same column definitions,
including the style-45 tail through XFD, and the same row-style assignment.
The sheet's only changed top-level XML element is sheetData. SheetShrink
removed 91 empty cell records. The removed cell styles and the inherited
column style all have fill 6 and border 0 in the respective style tables.
The drawing, chart and media member size/CRC inventories match.

An explicit native reference of By Country!A1:AB54 and XFB50:XFD55
finished in 17.52 seconds. Each pair had equal measured geometry and
equal rendered pixels. The first range includes the declared cell
dimension with context. The second samples the far-right inherited fill.
These two observations do not resolve whole-grid or other-sheet coverage.

- SheetShrink output SHA-256 `7bc32656bf8739dc3e876f240dad6c4d3bad79fec80c81ac33387d1714e622da`
- Native reference SHA-256 `93112746436eae2d7c3fd6434f5566e61b3ff6ad009e4b0875a9b666b6869e03`

A separate ordinary Inquire attempt used the same prepared original and
candidate as the pending Raspberries slot. It ended with unresolved
whole-grid coverage on By Country!A1:XFD1048576. The failure is distinct
from the XLStylesTool opening exception. It also remains separate from
the bounded, equal SheetShrink reference above.

Inquire failure receipt SHA-256
`36fe36f0b7a6cf1fdb393ad6e1a142d9a0eb1c8a54d08026fc74f8d8d2de4f67`.

## Enron SheetShrink chart findings in the top-49 selection

The current-output identity audit selects five Enron workbook observations
with six chart findings. Re-decoding their retained image pairs yielded
the counts below. All six pairs have equal image dimensions and no
recorded geometry difference.

| Frozen rank | Sheet | Chart | Changed RGBA pixels |
|---:|---|---|---:|
| 7 | Run Query | Diagramm 3 | 19 |
| 7 | Run Query | Diagramm 4 | 9 |
| 9 | Hedge Example | Chart 1 | 2 |
| 12 | Results | Chart 1 | 3 |
| 17 | Hedge Example | Chart 1 | 26 |
| 23 | Chart | Chart 1 | 11 |

The chart dependency gate did not certify unchanged dependencies for
these six findings. Small counts are compatible with prior rendering
variation controls, but do not establish that cause for any individual
chart. The raw findings remain visible. Source report SHA-256 values,
in rank order, are:

- `7e9edd85057da769be7a75df7ab26d480258832fd417988da9a54a51d5d9cc7c`
- `7f99d99d60b8a5ea6f502ccbcd077716db593a483cedbecddd1c94de48307d8a`
- `990e281b92163b4579ca33145961f878fe2213484964aee3a0d1a701c7d92a68`
- `a7f2526fe95c5afc6a22b453a25d083805f6f152bf98f62ef2989b1dd808757f`
- `9a091bdb450f45fdc33e883630aa47a4f94a3084c809497819a4e7b01ff580a9`

### Enron chart controls with identical workbook members

Separate conservative captures repeated the five workbook inputs above.
In each control the second package differed only in its ZIP comment.
Every workbook member matched by SHA-256 before native opening. The five
controls captured ten charts. Two changed in the same chart identities
as the corresponding SheetShrink observations:

| Frozen rank | Sheet | Chart | Changed RGBA pixels in control |
|---:|---|---|---:|
| 7 | Run Query | Diagramm 4 | 2 |
| 12 | Results | Chart 1 | 7 |

The other eight control captures were equal. Both changed controls kept
the same image dimensions and reported no geometry change. After opening
and rebuild, the changed package members were confined to workbook XML,
core properties and, in rank 7, the calculation chain. These controls show
that differences of this scale can occur without a workbook-member
transformation in these two charts. They do not establish that the
original pixel masks have the same cause. All six original findings
remain in the report.

A coordinate-by-coordinate comparison of the retained masks found no
overlap between rank 7's nine original changed pixels and its two control
pixels. At rank 12 all three original changed coordinates occurred among
the seven control coordinates. Both comparisons used the same image
dimensions. This measures coordinate overlap independently of the color
change at those coordinates.

### Enron rank 30 raw opening exception

The top-49 identity audit has no eligible comparison for rank 30,
workbook `632d959be36ff9610d832c34b55c60ae4691f09d727f7f575c3c467e27b5ac0d.xlsx`.
An original-only native opening control stopped at an Excel
`repair_required` dialog and confirmed shutdown without accepting it.
Separate COM opening controls also returned HRESULT -2146827284 for
both the original and the historical SheetShrink output.

The XLS reconversion control produced an alternate XLSX successfully
(SHA-256 `8836f59174c865cfde29dffe99ed98aa9b768694352d6d962dd2cb166988c7c0`).
It would require an independent opening check and a new comparison
using that input identity. The conversion receipt supplies no visual
coverage for the original selected workbook.

- Native failure receipt `3840d9c90043beef104fd553fd950e0f95fe432c387f470f24364c8d02da6b49`
- Reconversion receipt `a7b588fed3a3a597271378b2fdedd5d2d17930956f0d985834a877268f71f9e6`

A later bounded test used the reconverted XLSX as input to the current
SheetShrink binary. Cleanup produced a candidate (SHA-256
`c571513855f899322bca03727b15a3e6747d60e3cd79ab235b653d340448e4a7`),
but the candidate's native opening lane encountered `repair_required`.
A second test used the reconverted input in both lanes and reproduced
`repair_required` in the original lane. Neither test accepted repair
or reached the two planned reference captures. The reconversion does
not provide an eligible comparison baseline under this opening policy.

- Reconverted pair receipt `96b88e38951c92062858e6380e46f3a3284f561cc71208bba936dcb1bfdaf93c`
- Reconverted input-only control receipt `220509880d601f9353a72ee02f53029fca95ece10ec7102251efa704e6d82dee`

The independent QT/RR 2015b Inquire attempt produced the same
90,112-rectangle plan as the first QT/RR workbook. Both recorded plan
indexes have SHA-256
`b517482415fd8e730fa7ef137c46ba6c3653ea7a1aca15f3a4f019b50d898e08`.
The pre-grid inputs match byte-for-byte as well. In a live snapshot,
all 453 original-side and 454 candidate-side PNGs already written by
2015b matched the SHA-256 of their namesakes in the earlier run.
The workbooks retain distinct identities and incomplete observations.
This sample supports storage deduplication for verified matching bytes.
It supplies no basis for skipping the remaining native captures.

A full member and worksheet audit of the first QT/RR rebuilt pair
found no added or removed ZIP members. The styles and shared strings
are byte-identical. Changes outside the worksheets are confined to
document properties and the workbook's AlternateContent and
revisionPtr elements. Within the worksheets, only sheetFormatPr
(12 sheets) and sheetData (11 sheets) differ. Each affected sheet
adds three empty styled cells. No cell payload changed. The only
changed row attributes are s and customFormat on the eleven first
rows. Explicit columns, merges, conditional-format elements and
drawing references are unchanged.

This narrows the work needed for a defensible plan. It must still
qualify the effective default widths and heights and preserve the
native inherited border extent. The existing three native samples
only establish equal geometry at their measured coordinates.

An independent reference on the 2015b workbook repeated the three
SHR1 DE probes in 75.15 seconds. The three region receipts match the
first workbook's receipts exactly, including geometry, image hashes,
and the missing border at XFD. The rebuilt OR packages across the two
workbooks differ only in docProps/core.xml and xl/workbook.xml, and
the same holds independently for the rebuilt SDR packages. The
changed XML children are modified in core.xml and AlternateContent
and revisionPtr in workbook.xml. Workbook roots and all worksheet,
style and shared-string members match between the corresponding
states. The distinct source identities and incomplete ordinary
comparison records remain separate.

2015b native reference SHA-256
`876bdc2e7eadc76fa38f8ec3bcd4e7dc0e132dc5b01548c8952cfd4f209a700b`.
