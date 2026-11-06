# Local row-font masks

## Observed regressions

`CAPs Acrolein Elemental Analysis_1eb445809fa5.xlsx`, `Supplemental Table S1`,
row 46 changed from 12.75 to 15 points. Its empty D46 and I46 cells explicitly
use the default Arial 10 font over Arial 12 columns. Restoring either cell alone
was insufficient. Restoring the original row, or preserving both local masks,
restored the height before save, after save and after rebuild. A span-only
control did not restore the height. The production-style correction also produced
24 compared regions with no pixel or geometry differences.

`PAH Published Dataset_Data in brief_5560dcdfd33b.xlsx`, `Additional Sorbent
Tubes`, row 4 has Arial 10 cells over otherwise implicit Calibri 11 columns.
Removing these exposes a new row font. With the local masks retained, all
measured heights in rows 1 to 30 match in all three phases. Its `Reaction
Products` row 5 also needs a representative for an explicit Calibri font whose
XF omits applyFont; the inherited row XF uses Arial. After preserving one such
representative, all 11 selected regions matched without pixel or geometry
differences in a full paired-rebuild comparison.

`zhang2023-u1483-alldata_78e2f6039536.xlsx`, `Data_MAR2`, has automatic rows
without an ht attribute. Preserving local default-font masks made rows 1 to 25
match exactly before save, after save and after rebuild. In particular, rows 1
and 2 remain 16.5 points instead of increasing to 17.25.

## Bloat constraint

An implicit-font exception is needed only when removing it introduces the
previously absent default font. The original row is scanned once on demand.
Existing default-font cells or uncovered gaps allow ordinary representatives.
Unknown or merged layouts do not establish this proof.

Dense automatic rows can promote an existing full XF to the row, with the same
bounded gap restoration used for fixed-height rows. No new font or style record
is introduced. Automatic merged rows are excluded from promotion.

A synthetic native workbook contained two rows spanning all 16,384 columns in
Arial 24. One row had an implicit default-font gap. Both kept 30-point effective
height before save, after save and after rebuild after compaction. The visible
anchor below remained at its original measured position. This verifies effective
heights, not full-workbook pixel identity.

## Evidence and reproduction

The existing `../nla-row-height-control/run.ps1` supports worksheet and row bounds.
All native probes used owned private desktops and disposable inputs. Every
successful height control confirmed both Excel processes closed.

Local evidence is under
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp` in
`cap46-local-font-height-proof`, `pah-bounded-font-proof`,
`zhang-bounded-font-proof`, `automatic-dense-font-proof/native` and
`height-regression-controls`. These machine-local corpus artifacts are not
committed.
