# Hidden-column font masks and one-pixel height changes

## Reproduced regression

`DE-EE0007347 M2.4 H1 LCOE Model PD v1.0 03-06-2017_57e01cc50096.xlsx`, group
`0d8586700aaeff9826a2`, has hidden columns F:H on `Installed Cost Overview`.
Their column style supplies bold Arial 11 (font 13). Empty cells supply regular
Arial 11 (font 26) instead. Both fonts have the same point size, but their
measured row-height contributions differ.
Although visible cells also use Arial 11, they cannot mask the hidden columns.

The previous cleanup removed those local masks because their font was already
represented elsewhere. Original row 14 rendered at 14.25 points; Default at
15. Row 35 rendered at 15 points; Default at 15.75. Both differences survived
opening, saving and full rebuild in the paired native control.

The correction preserves each local mask only when its removal exposes an
inherited font absent from the original row. Font queries consider explicit
cells and unmasked column gaps and are cached per row. Hidden-column cases reuse
the same mechanism as implicit-column masks. Existing tests still verify that
hidden font representatives are deduplicated when their inherited font already
exists. Styled and styleless local masks have focused regression coverage.

## Native verification

With the correction, rows 1 to 40 have identical measured heights before save,
after save and after rebuild. The full paired-rebuild comparison then completed
with all 18 selected regions matching. No pixel or geometry differences remained.
The source corpus package was preserved. Every successful height control
confirmed both owned Excel processes closed.

Disposable evidence is under
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp` in
`lcoe-height-regression-proof`, `lcoe-hidden-mask-fix-proof` and
`lcoe-hidden-mask-full-proof`. Reproduction uses the existing private-desktop
height-control launcher with `-Sheet 'Installed Cost Overview' -StartRow 1 -EndRow 40`.
These observations retain the selected-region coverage limitations.
