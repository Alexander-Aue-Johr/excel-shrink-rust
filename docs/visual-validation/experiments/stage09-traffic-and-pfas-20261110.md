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
