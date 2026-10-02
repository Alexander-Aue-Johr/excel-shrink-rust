# Visual-validation decisions

This file records decisions that would otherwise be easy to forget after the
experimental code has been simplified or removed.

## Worksheet cell/content bounds

**Status:** accepted

Use the native Rust regression analysis to determine the last relevant worksheet
cell bounds.

Do not add Excel `Find`, `UsedRange`, or a second Excel-side scan merely to find the
rightmost/bottommost relevant cell.

**Revisit if:** a real workbook demonstrates visible cell content that the Rust
regression analysis does not include.

## Drawing bounds implementation

**Status:** implementation choice pending extended timing

This experiment is an engineering benchmark for the Excel Shrink visual-regression
pipeline. It is not intended as a paper experiment. The goal is to find the fastest
reliable way to obtain Excel drawing bounds before screenshots are captured.

Compare the available execution models over the same corpus and intended geometry:

1. VBA running inside Excel.
2. C# / Excel-DNA running inside Excel.
3. C# / VSTO running inside Excel.
4. Office.js running in Excel's web add-in runtime with batched shape loading.
5. external Windows PowerShell -> Excel COM.
6. external Rust -> Excel COM.

Geometry correctness is validated against VBA using canonical `right` / `bottom`
data with timing removed. Canonical geometry is compared in memory and diagnostic
copies are persisted only for actual mismatches.

The current four-way result set shows geometry agreement across VBA, Excel-DNA,
PowerShell COM, and Rust COM for the tested corpus. Rust still has a documented
visibility-filter caveat because the current `excel-com` dependency does not yet map
`Shape.Visible`'s `MsoTriState` / `VT_I4` representation to the Boolean API expected
by the helper.

Select the implementation after VSTO and Office.js have also been measured and the
fastest candidates have been repeated enough to rule out obvious run-to-run noise.

Evidence:
[`shape-bounds`](experiments/shape-bounds/)

## Text overflow

**Status:** accepted

Do not calculate exact text-overflow bounds. Use a reasonable minimum screenshot
width instead.

This intentionally trades some empty screenshot area for substantially simpler
capture-bound logic.

**Revisit if:** real regression cases show relevant visible text being clipped by
the chosen minimum width.

The `range-image` experiment deliberately includes exact-overflow strategies D/E/F as
an engineering investigation. Running those strategies does not change this production
decision until their cost/reliability has been evaluated.

## Screenshot persistence

**Status:** accepted

Persist `CopyPicture` results as image files.

An in-memory/GDI comparison can be fast, but persistent screenshots provide both a
reusable cache and inspectable failure artifacts.

**Revisit if:** persistent image I/O becomes a demonstrated bottleneck that is not
offset by cache reuse.
