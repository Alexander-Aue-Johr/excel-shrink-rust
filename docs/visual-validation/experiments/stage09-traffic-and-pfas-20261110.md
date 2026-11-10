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
