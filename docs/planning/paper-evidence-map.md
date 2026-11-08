# Paper arguments and evidence map

Working design dated 2026-11-08. This map defines manuscript content and identifies
missing experiments. A proposed claim enters the results section only after its
listed evidence has been inspected and its population and environment recorded.
The prospective protocol is [evaluation-protocol.md](evaluation-protocol.md).

## SheetShrink

The central question is whether deterministic package cleanup can reduce the
cost of using spreadsheets while preserving their required behavior. Explain the
preservation contract before presenting removal rules. The opening-cost argument
requires measuring cleanup plus opening, including cases with little bloat.

| Proposed argument | Present evidence | Required evaluation |
| --- | --- | --- |
| Compact sentinels preserve required row formatting | Core row promotion and inherited-style tests; preservation specification | Native mutation cases and independent sampled whole-area comparisons |
| Hidden content remains when meaningful dependencies require it | Reference-scanner tests and historical hidden-tail fixes | Cross-sheet, name, chart and conditional-reference fixtures with native controls |
| Default removes irrelevant structure without rewriting shared tables | Current cleanup implementation and specification | Package-member accounting, values/formulas and source-dependent output checks |
| Cleanup can pay for itself before a workbook opens | Historical experiments require separate provenance review | Controlled matched original-open versus cleanup-plus-cleaned-open trials |
| Small inputs incur acceptable extra cost | No accepted aggregate result yet | Include low-bloat files and report absolute overhead and distributions |
| Byte-oriented processing and scheduling explain throughput | Implementation and historical traces | Controlled single-thread, parallel and synthetic workload experiments |
| Default preserves tested visible presentation | Enron and Data.gov development observations, raw findings and manual ledger | Frozen coverage totals, independent reference sample, reviewed unresolved cases |

Recommended initial section order is Introduction, Contract and Bloat Model,
Cleanup Decisions, Byte-Oriented Execution, Evaluation, Related Work, and
Limitations and Conclusion. Explain the small set of mechanisms that carry the
argument in the main text. Put the exhaustive rule inventory in versioned docs.
Historical Python tools belong in a labeled evolution experiment because their
outputs and preservation rules differ from the current subject.

Plan a motivating figure showing input structures, a minimal retained sentinel,
and an end-to-end opening-cost breakdown. Until matched measurements exist, use
the structural example without numeric speedup annotations. Prefer a method
figure to an Observatory screenshot for this paper's first-page argument.

## DeltaSieve

The central question is how much native visual work can be avoided while still
detecting relevant spreadsheet transformation effects. Distinguish selecting a
potentially changed region, observing unequal pixels and attributing a defect.

| Proposed argument | Present evidence | Required evaluation |
| --- | --- | --- |
| Selection reasons are inspectable and dependency-aware | XML, style identity, conditional-formatting and chart logic and tests | Independent mutation coverage by supported mechanism and fallback |
| Effective properties avoid false candidates from ID renumbering | Style/shared-table comparison implementation | Explicit renumbering and equal-ID changed-property counterexamples |
| Native capture verifies selected rendered states | Paired-rebuild pipeline and completed native smoke tests | Frozen corpus outcomes and renderer/environment metadata |
| Selection substantially reduces capture cost | Production telemetry offers observational evidence | Matched selected/full-reference sample with counts, pixels and phase timings |
| Fallback preserves honest uncertainty | Existing limited-coverage and failure receipts | Unsupported dependency and timeout cases with visible coverage accounting |
| Reuse avoids repeated compatible work | Tested standalone pair reuse and completed-study resume | Changed-input, changed-renderer and changed-options invalidation tests |
| Portable evidence supports independent inspection | Static export smoke tests | Browser exploration of a real difference, paths/privacy audit and offline package checks |
| Rendering variability can be separated from defects | Individually inspected chart and extent cases | Repeated same-input controls; keep unresolved cases explicit |

Recommended initial section order is Introduction, Comparison Contract, Candidate
Selection, Native Execution and Evidence, Evaluation, Related Work, and Limitations
and Conclusion. Group Related Work around spreadsheet testing and slicing,
document comparison, change-impact analysis, and native rendering oracles. Verify
each cited system's actual coverage before assigning it to a baseline category.

Plan a first-page structural example with a source change, affected dependency
region and the smaller capture plan. An annotated Observatory detail may support
the evidence section. Include a first-page interface image only if its labels
and central selection insight remain readable at final two-column size. A replay
is a demonstration artifact; timing and detection claims require the underlying
measurements and independent controls.

## Separation and release boundaries

SheetShrink's evaluation may use DeltaSieve while summarizing its selection rules,
sampling, native environment and limitations within the cleanup paper itself.
DeltaSieve's evaluation must include transformations beyond successful cleanup,
including injected visible changes. Each manuscript must remain understandable
without another submission's paper ID or acceptance.

Keep the live Enron counter out of static manuscript results until a dated cutoff
is frozen. Counts of completed attempts, successfully compared workbooks,
unchanged packages and skipped files have separate denominators. Replay and
interface polish do not substitute for selector coverage or cleanup correctness.
