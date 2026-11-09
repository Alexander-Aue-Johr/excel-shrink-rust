# Paper arguments and evidence map

Working design dated 2026-11-08. This map defines manuscript content and identifies
missing experiments. A proposed claim enters the results section only after its
listed evidence has been inspected and its population and environment recorded.
The prospective protocol is [evaluation-protocol.md](evaluation-protocol.md).

Both fresh LaTeX projects now contain working abstracts, contract and method
sections, editable first-page method figures, verified bibliography entries,
Related Work and explicit evaluation gaps. The paper repository records the
expanded drafts in `fa8f8c3`, figures in `6feb8e2`, literature updates in
`e37a90b` and the author-review/build checklist in `f9dbf5d` on `icde2026`.
Both four-page working PDFs compiled with TeX Live and all eight rendered pages
were inspected. Final evaluated results, complete evidence-backed manuscripts
and author revision remain open under Task 14.

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

## Verified manuscript refresh on 2026-11-09

Paper commit `10acc6d` updates both independent evaluation sections with the
imported Enron input frame and the completed initial Destatis Check Performance
preparation. The Enron import contains 34,065 distinct candidate hashes and
8,469,579,995 payload bytes, with externally generated extraction provenance.
The import archive hash is
`cd7c91e28c833a2a7b78673ece21223317c30379d37009ebfc95f9fef1f47c22`.
Candidate recognition, conversion and visual outcomes remain separate stages.

The initial Check Performance preparation attempted 1,182 inputs, retaining
1,016 prepared outputs and 166 failed entries. Its receipt hash is
`619553532d01a784657b6c2f49367ae9cfe669cd3a59a666d1013785de523887`.
These counters describe preparation, with visual baseline validation still open.
The source investigation tracks subsequent diagnostic recovery independently.

DeltaSieve's draft additionally reports the fixed-area OLE frame/preview controls
and the corrected three-sheet zero-area text-box controls. Their exact source,
mutation and reference identities are recorded in the versioned experiments.
They support bounded development observations. Independent corpus-level detection
and matched selected/full-reference cost experiments remain open.

Both projects compiled twice without undefined-reference or overfull-box warnings.
All ten rendered pages were visually inspected. The SheetShrink working draft
has four pages and DeltaSieve has six. These manuscript updates leave Task 14's
evaluation freeze, full claim audit and author revision gates open.

Follow-up paper commit `652e7ac` incorporates the completed recovery classification
for all 166 failed preparation source identities. It records 165 Name Conflict
observations, the recurring Interactive-property automation error and a successful
separate positive control. Both PDFs were rebuilt twice and re-rendered. The six
unchanged page images matched the previously inspected versions, and all four
changed pages were inspected. Final PDF hashes for this checkpoint are
`5fa5e6a1100fae189c7e29996ec243ef84c3309a9ba0ed46562f6a2ba4eb07a7`
for SheetShrink and
`62c7e73a597a1ad49f3398b9c60906ee20e3cacc569845d6fb7a382a3b3b714e`
for DeltaSieve.
