# Blank capture margins and the evidence atelier, 4 November 2026

The targeted comparison includes one extra row and column around content. Shrink
can legitimately change their dimensions after deleting exterior layout cells.
Those empty margins previously made a workbook a difference even when the
content pixels matched.

## Comparison rule

`targeted-content-grid-v4` keeps the extra captures, including overflow text.
Office.js measures both the complete capture and its intersection with the
unpadded union of content bounds, in the same existing geometry synchronization.
Gridlines are disabled consistently; explicit cell borders remain visible.

A geometry difference is ignored only if:

- Both states have a positive-area, identical measured content core.
- The capture origins match and both captures fully contain that core.
- All top-left-aligned nonblank pixels match, allowing white padding only at
  the right and bottom. No rescaling, tolerance or shifted-image matching occurs.

The report preserves raw geometry, images and `blank_margin_ignored`. Missing
core measurements, moved content, ink in the extra area and changed content-core
geometry retain the strict difference verdict. Chart comparisons are unchanged.
Only fingerprints remain cached; decoded images are released after hashing.
Older capture policies cannot satisfy a resumed v4 comparison.

Relevant observations were Enron `Deals` in
`native_000%2F3.608401.CDOZVCACXNQKWR5NDN3SG3Y4LG1AO4FUA.1.xls`
and `Top 100 Exposures` in
`native_001%2F3.384235.FG0FUFRY0QVIIBREKZKPSRNLVKM1QBSIB.1.xls`.
The latter also exposed meaningful hidden values removed by the old shrinker;
that independent issue is covered by the hidden-constant-tail regression tests.

## Observatory

The runner publishes its targeted mode explicitly. That mode hides unused
hash-sweep and legacy tile-scene controls. Active preparation filenames appear
as individual chips. Non-applicable zero-count KPI boxes are omitted.

`Checked: no differences detected` means a completed targeted comparison without
a finding, or an existing complete visual-identity verdict. It does not count
skipped, excluded or incomplete workbooks and does not certify whole-workbook
visual identity. The separate differences KPI opens the result list; selecting
a difference opens the fullscreen atelier. The workbook heading can reopen it.

The atelier reads retained portable reports and PNGs, with phase labels O, SD
and SDR. It shows one image pair at a time, with zero gap and a common display
scale. Original pixels remain available through the 1:1 toggle. Closing aborts
pending requests and releases the image elements. Prepared Original/Default
inspection and download actions remain available in result lists.

## Verification

Focused Rust tests cover blank right/bottom margins, changed core geometry,
missing measurements, moved origins and overflow ink. Existing targeted report
contracts, progress tests and control-room tests are also exercised. The Office.js
bridge test verifies that core measurement adds no synchronization.

A headless Edge check against a small copy of real run evidence verified the
KPI lists, Original and Default downloads, native fullscreen, a zero-pixel image
gap and no uncaught JavaScript errors. This UI check does not regenerate images.
A previous oversized whole-CLI test invocation was stopped; its local-corpus
profiling tests are outside this change's focused verification.
