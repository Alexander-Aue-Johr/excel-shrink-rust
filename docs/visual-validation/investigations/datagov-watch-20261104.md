# Data.gov watch findings, 2026-11-04

The validation campaign is ongoing. These observations distinguish product defects,
capture artifacts and unavailable baselines. Raw images and XML transitions remain
in the run evidence.

## Confirmed Default regression

`Fig 10_New 10-29-19_b3bff94c7378.xlsx`, sheet `Fig. 10`, centers a heading across
D2:G2 with `centerContinuous`. Default removed F2 and G2 and shifted the heading.
Restoring only G2 did not restore the layout in a native comparison. The new
content-seeded contiguous-span rule retains the necessary cells. The fixed release
output is byte-identical to this 11,668-byte input. Isolated centering formatting
without content remains removable.

An offline style inventory checked all 8,623 prepared bundles in 101.05 seconds,
without errors. It identified 144 possible affected workbooks. Their records and
old bundles were backed up before selective invalidation. Unaffected preparation
builds remain explicitly approved. Capture policy v8 also includes these spans
in the visual envelope; earlier reports retain their original coverage versions.

## Capture boundaries requiring follow-up

`DataTables2And3_313f11e1abbf.xlsx` and
`Sec4.2 CO2 Result Reasonableness Check_050b0f8ad9d0.xlsx` have matching measured
content cores but different outer context dimensions. Their images are respectively
637x268 versus 653x267 and 580x239 versus 560x239 pixels. Every pixel in each
common top-left-aligned rectangle matches exactly. Overflowing text reaches the
unequal capture edges. These observations indicate a viewport/cropping artifact;
the raw differences remain unresolved by the automated verdict. No blanket pixel
tolerance or automatic acceptance of unmatched overflow ink was added.

## Blocking compatibility dialog

The Original of
`average-expenditures-per-patient-by-healthcare-priority-group-fy2000-to-fy2017__d6c7a4a6-2939-4d64-a2c7-973910bbfc7f_57f62be347d7.xlsx`
opened and sent add-in heartbeats but its save waited for a modal `NUIDialog`
named `Microsoft Excel - Compatibility Checker`. A native repeat with the new
classification promptly produced `compatibility_confirmation_required`, including
phase, owned PID and observed title. No prompt was accepted. This is an unavailable
saved baseline, not a successful comparison or a demonstrated Shrink defect.

## Transient opening failure

`odp_hcp_flu_vaccination_by_county_2020-21_datadictionary_44db6540ef69.xlsx`
had one 180-second opening timeout. Original reached add-in readiness while Default
did not start a WebView. An isolated repeat of the exact same inputs completed.
This establishes recoverability, not a proven root cause. The main run returns to
four groups while further opening failures are investigated.

See also the [registration accumulation investigation](addin-registration-accumulation-20261104.md)
and the [blank-margin contract investigation](blank-margin-contract-20261104.md).
