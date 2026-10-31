# Protecting referenced cells during cleanup

Excel Shrink may remove content from hidden cells only after scanning workbook
references. A cell whose value can affect a formula must be retained even when
its row is hidden. The scanner records protected cells and rectangles; the
deferred cleanup removes a candidate only when it is outside those sets.

## Shared formulas

An XLSX shared formula stores the expression at an anchor cell and marks its
followers with the same shared index. The followers do not repeat the text.
For example, an anchor at `D166` with range `D166:J166` and expression
`D91/D64*100-100` makes `E166` refer to `E91/E64`. Scanning only the stored
expression protects `D64` but misses `E64`.

The scanner now computes the smallest enclosing rectangle for each reference
as the anchor expression moves across the declared shared-formula range.
Relative axes move; `$`-fixed axes remain fixed. It inserts a rectangle once
per reference instead of expanding every follower formula. Sheet-qualified
references are assigned to their target sheet; unresolved sheet names retain
the existing workbook-wide conservative scope. If a shared group is incomplete,
conflicting or moves a reference outside Excel's grid, the scan reports it as
unsafe and the workbook retains reference-sensitive cleanup candidates.

The implementation is in
`excel-shrink-core/src/reference_scanner/shared_formulas.rs`. The focused tests
in `excel-shrink-core/src/reference_scanner_sheet_scope_tests.rs` cover the
Destatis pattern, absolute axes, escaped cross-sheet names and unsafe groups.
The CLI applies the resulting protection in
`excel-shrink-cli/src/pipeline.rs` (`collect_safe_candidate_causes`).

## Observed regression and evidence limit

The Destatis workbook `verkehr-aktuell-xlsx-2080110.xlsx`, worksheet `1.1.5`,
has a hidden row 64 containing `E64 = 4232.35`. Its Default output removed
`E64`, while the shared formula at `E166` still used that value after Excel
recalculation. The local study record `db466233adf36879eb65` reported 3,956
post-recalculation value differences and visible `#DIV/0!` errors. Restoring
1,999 deleted hidden cells in memory and recalculating removed all 14 inspected
image differences. That intervention isolates the deleted hidden-content set;
it does not prove that every restored cell was individually necessary.

A local shrink rerun with the shared-formula fix retained `E64:I64` in the
Default output while continuing to remove other cells. The corrected output
has not yet been through the full Excel recalculation and visual comparison.

This rule is a preservation decision, not a claim that all Excel formulas or
external data behavior are fully modeled. The paper should distinguish the
scanner's documented preservation rule from the measured corpus outcomes and
cite the exact code revision and validation artifacts used for each result.
