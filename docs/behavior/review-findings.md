# Review findings, evidence and unresolved decisions

Review baseline: `eac0300`, 2026-11-02. Corrections below are part of this review
series. This is an engineering review, not independent certification. It does not
convert scoped visual observations into a universal preservation guarantee.

Corrections: `5822800` (defined-name scope), `4d9098f` (shared-string header
eligibility), and `f863972` (completed-report contract).

## R1. Corrected: defined-name scope depended on attribute order

The old workbook-name regex captured `localSheetId` only when it appeared after
`name`. Two names with scope before the name could therefore both be treated as
workbook-scoped and an alias on another sheet wrongly removed. Single-quoted
identifying attributes were also not recognized consistently.

The new tests reproduced both failures before the correction. Identification now
uses the existing attribute scanner, independently of attribute order/quote style.
Missing/ambiguous identifying attributes prevent cleanup. Self-closing entries
cannot consume a later sibling's content. Existing alias-name policy is unchanged.

Relevant tests: `workbook::scope_tests` and existing workbook-cleanup tests.
The core library suite passed: 239 tests passed, one pre-existing ignored test.

A read-only metadata inventory of the current prepared Data.gov records found
**zero Default results with changed workbook XML according to their stored
CRC/length evidence**. The reproduced bug does not by itself establish that a
current retained Data.gov output is affected. Those results retain their actual
old producer identity; they are not relabeled as outputs of the corrected core.

Remaining question: the alias rule checks name/scope, not formula-body equality.
The specification records that distinction rather than inventing a stronger
guarantee. Whether differently defined aliases should ever be removed needs an
explicit preservation-policy decision and dedicated cases.

## R2. Corrected: consumer could accept missing result fields as no differences

The native workflow already failed on missing capture partners or unreadable
images. However, the archive consumer independently classified a completed-looking
JSON report by iterating optional arrays and testing fields against `true`.
Missing arrays/flags could therefore appear as no differences at that boundary.

The new report contract requires five/nine exact requested pairs, explicit unique
region coverage, equal region sets, image references and Boolean verdicts. It also
requires chart result arrays and the explicit chart difference flags. Malformed,
partial or duplicate structures fail before classification/evidence deletion.

This is a real fail-open structural weakness, **not evidence that the producer
actually lost past results**. The focused tests exercise missing fields/pairs,
duplicate/mismatched regions and successful difference propagation. A read-only
audit accepted **84 retained completed local reports**, with classifications
matching their stored records. No records were rewritten by that audit.

Transfer integrity and resume metadata predicates remain different boundaries;
they are not a fresh execution of the image comparison. A separate audit remains
appropriate for historical/imported results whose producer predates a new contract.

## R3. Earlier corrections that motivated this specification

These were identified before this review, and are not new discoveries here:

| Issue | Corrected behavior / evidence boundary |
| --- | --- |
| Concurrent launcher journal append collision | `af04c55`: per-path mutex around ownership append/read; parallel Excel work remains parallel. Tested with 12 processes and 300 entries plus cleanup safety tests. |
| Default-only targeted result requeued for unused profiles | `c0599be`: later preparation check uses effective Default profile; explicit original-identity subsets can resume in the same output. |
| Missing Check Performance byte sizes after work cleanup | `6cdac44`: preserve `workbook_bytes` before disposable XLSX deletion. Old missing values remain unavailable. |
| PID probes displayed as Add-in heartbeats | `eac0300`: cyan chip for liveness; actual received Office.js signals for the heart curve, with exact identity binding. |
| Preparation counter bouncing backwards | `e48c313`: queued preparation retries become pending together; not a change to scientific comparison outcomes. |

The shared `windows-owners.jsonl` is process ownership data, not a shared report
into which all comparisons concurrently append scientific results. Actual report
attempts have separate UUID directories and durable per-workbook records.

### Additional correction: shared-string header eligibility

The broader pipeline suite exposed an existing failure in
`prefixed_spreadsheetml_is_skipped_without_publishing_output`: a prefixed
shared-string representation was not excluded with whitespace cleanup disabled,
because that part was classified as copy-only. The shared-string root is now
included in the bounded header check even in that mode. Unrelated copy-only
custom XML remains excluded from that preflight. No full shared-string-body pass
or redundant worksheet scan was added. The pipeline test selection then passed:
48 tests passed, two pre-existing explicit/benchmark tests ignored.

## R4. Remaining scalability boundary: accumulated changes and raw candidates

Cell fingerprints are disk-streamed, but change evidence and initial candidate
sets are still accumulated in memory. Removing worksheet byte/cell limits did not
make every downstream structure streaming. A large union/content rectangle can
contain many internal formatting-only cells.

The local FRPP priority case exposed roughly 32 million intermediate candidates
and about 19 GiB private memory during analysis. That number was not a final
capture-job count. Gzip audit files keep durable report size manageable but do
not eliminate the earlier in-memory construction cost. A full streaming audit
writer plus early grid aggregation is a possible follow-up, provided all change
evidence, dependency selection and late backfill remain equivalent. Do not
introduce a hidden candidate-count skip as a performance fix.

Effective row-height verification can also remain expensive when every row has a
different height. Time/budget exhaustion must remain a failure rather than a
sampled equality verdict. The broad automatic-height scan is currently deliberate.

## R5. Abstraction and coverage boundaries requiring owner review

1. **Original rendering baseline:** O is already opened/saved, not a screenshot
   of a never-saved raw input. Raw XML transitions remain available as fingerprints.
2. **Check Performance causality:** CP-before is not a separate live pixel lane;
   direct CP differences include that copy's extra opening/saving path.
3. **Non-chart objects:** arbitrary drawings/text boxes/pictures/VML are not
   exhaustively selected by the targeted algorithm. Legacy support is not proof
   of targeted coverage. This remains an explicit report limitation.
4. **Content clipping:** exterior formatting-only differences and content-free
   sheet regions are intentionally not pixel-certified. Finite padding cannot
   establish absence of arbitrary long overflow outside its neighborhood.
5. **Volatility/external state:** XML/rebuild controls identify overlap, not
   exclusive causes or deterministic behavior of RAND/NOW/external sources.
6. **Name alias policy:** same recognized name/scope is used without comparing
   their formula bodies. This is narrower than proving semantic duplication.
7. **Parser scope:** the fast shrinker is not a general normalizer for all valid
   SpreadsheetML/XML serializations. Unknown forms and unavailable drawing bounds
   deserve conservative case-specific review.
8. **Telemetry:** raw candidate counts and selected capture totals can share
   progress fields at different phases. A discovery count must not be explained
   as already scheduled millions of screenshot jobs.
9. **Existing artifacts:** source fixes do not retroactively regenerate prepared
   outputs or establish that older measured outputs came from the new implementation.
10. **Import trust boundary:** verified artifact hashes establish transfer
    integrity. They do not prove the producer's selection coverage or semantic
    correctness; portable and historical absolute-path references have distinct uses.

These entries are not excuses to label missing coverage successful. They state
what an independent reviewer must not infer from the current result labels.
Policy-changing extensions need a new capture-policy identity or an explicit
compatibility/revalidation decision. This review does not silently replace the
current selected-region strategy with full-workbook screenshots.

## R6. Real comparison findings observed during the review

The priority investigation uses the original Data.gov archive, output and stable
identities, reusing measured Default preparations. The following are reported
observations, not proven root causes:

- **Adage oilprice main results** (`0a7be2a4c915f8824b39`): direct Default had no
  reported differences. Direct CP had three pixel/geometry-different regions;
  they persisted in the both-rebuilt CP comparison. CP's own rebuild showed no
  new differences. Raw input was 139,770,818 bytes; ordinary saved O 66,186,765;
  CP's own baseline 66,186,767 and optimized result 66,186,371. Attributing the
  entire raw-to-saved reduction to Check Performance would be incorrect.
- **Decentralized Life Cycle Cost Assessment** (`b52518d1b2ea787e165c`): direct
  Default had ten pixel-different regions (nine geometry differences), plus three
  chart differences. Both-rebuilt Default retained ten pixel/nine geometry
  differences but no chart differences. Original's own rebuild had three chart
  differences, so those chart observations cannot simply be assigned to Shrink.
  Direct CP had fifteen pixel/geometry-different regions and seventeen chart
  differences. Its own rebuild also changed charts. Root causes require inspection.
- **Raspberry dashboard** (`526ef2d226b7e07061d1`): failed with an unsupported or
  invalid worksheet row index. It is excluded from successful visual counts;
  no conclusion of visual equality follows from that failure.

The retained original-resolution difference images, compressed XML audits,
heights and separate pair labels are the evidence for further investigation.
No ad-hoc pixel tolerance or automatic dismissal of these findings was added.

## R7. Owner clarifications and remaining visual boundary gap

Requirements clarified on 2026-11-04 distinguish human-visible preservation from
rare accepted, imperceptible rendering variations and make Maximum/Too Aggressive
diagnostic artifacts. S10 records an optional future Excel add-in, separate from
the core. These are owner decisions, not claims of newly implemented features.

Inspected local source is `5e0e688`; the additional branch inspected is
`origin/experiment/hidden-constant-tails` at `45349c1`.
`targeted_comparison/cell_stream.rs::Writer::event` classifies content by
`v`/`f`/`is`/`t` markers. It does not resolve visible fills/borders for the envelope.
`content_bounds.rs::ContentBounds::select` clips to that envelope plus one cell
of margin. Therefore a styled-only sheet can be excluded and blank margin geometry
can create findings, contrary to the clarified visual acceptance requirement.

Commit `e960d6b` on the additional branch supplies measured core geometry and
`blank_margin_ignored`. It tolerates different trailing blank padding only when
core geometry matches and the anchored nonblank image evidence still agrees.
Its blank-pixel rule is opaque white or fully transparent. It does not enlarge
the envelope to include effective visible styles or implement perceptual-noise
acceptance. That correction was inspected, not ported or executed in this review.

The required implementation must resolve effective visible fill/border bounds,
retain their union across states, distinguish exterior geometry from relevant
geometry and keep chart/object coverage. Changing selection needs an explicit
capture-policy version and appropriate result-reuse handling. Acceptance cases
include a fill-only sheet, inherited fill cancellation, a border beyond the last
value, conditional visible formatting, deleted visible content, overflow across
the boundary, different blank trailing dimensions and a genuine internal height
change. White fill must not be globally discarded as blank image padding.

The current strict RGBA fingerprint records even a one-channel pixel difference.
It has no separate reviewed-rendering-noise outcome. Future monitoring/reporting
must distinguish raw differences, owner-accepted rendering variation, allowed
exterior geometry and unresolved/product defects without erasing source evidence.
The historical R6 observations remain raw findings until reviewed under that policy.

## Experiments supporting specific decisions

### Unstyled fill mask: PnP Sector Boilers, 2026-11-05

`PnP Sector Boilers Dataset_f6e5af66f1bb.xlsx`, `Fig 4 - SCR`, K6 is an
unstyled empty cell that clears the blue row fill. Ordinary cleanup retained
it, but tentative font-sentinel cleanup then removed the first such cell in
the otherwise empty fixed-height row. Commit `6e7c0c9` preserves this visual
role during both decisions without retaining unrelated font-only cells.

The regression test reproduced the lost K6 before the fix. Afterward the core
suite passed (256 tests, one ignored, plus four integration tests). A fresh
Default output retained K6 and L6. Native private-desktop comparison of rebuilt
Original and rebuilt fresh Default captured six candidate regions and eight
charts per state: all six regions, including `A1:AF74` on `Fig 4 - SCR`, were
pixel- and geometry-identical. Two separately captured energy-efficiency charts
each differed at one pixel (maximum channel deltas 26 and 19); these remain
separate findings, not an automatic pass. Both owned Excel processes closed.

Local evidence: `.tmp/validation-temp/pnp-fill-mask-proof/native-result/report.json`
on V:. This is candidate coverage, not an exhaustive full-sheet certification.
Cached outputs produced before the fix require a separate affected-case audit;
the successful fresh-output replay does not retroactively validate those files.

### Nanoplastics: equal rendering in a shared viewport, 2026-11-05

`Nanoplastics total metal concentrations_c4436fc6e645.xlsx`, `Intensity`,
`AO1:CB74` produced different image widths after empty-column cleanup. A
disposable copy of the current worker expanded only this range to `AO1:CD74`.
The expanded images were 4736 and 4781 pixels wide, both 1481 pixels high;
their entire common RGBA viewport was identical. Each complete old capture
also matched its exact crop from the expanded Original image. Thus the old
unpaired colored strip is reproduced in Original when its capture extends
far enough; it does not demonstrate a changed background in that viewport.

Evidence and the recorded override are under
`.tmp/validation-temp/nanoplastics-boundary-proof/` on V:. A report-hash-keyed
review preserves the original verdict and links this replay. Both owned Excel
processes closed. Production selection and comparison rules are unchanged;
this evidence is not a general exemption for different-sized captures.

| Evidence | What it supports; what it does not |
| --- | --- |
| [Parallel workbook opening](../visual-validation/experiments/parallel-workbook-open/README.md) | Four simultaneous startups had median 41.714 s versus 118.388 s sequential on that fixture. Supports trying independent starts together; not a universal Excel speedup. |
| [Native pipeline performance checks](../visual-validation/experiments/targeted-comparison/performance-20261102.md) | Original-resolution image caching, binary upload, compact audits, late backfill and closure checks on native cases. Single A/B capture runs are not paper-grade timing estimates. |
| [Reference protection](../methodology/reference-protection.md) | Shared-formula hidden-value regression and restoration evidence. Does not establish support for every Excel reference syntax. |
| Row-height experiments under `docs/visual-validation/experiments/` | Saved `ht` and effective height can differ, and uniform height runs can be cheap while alternating runs are expensive. Do not replace exhaustive selected-span checks with a few samples. |
| Namespace-header tests in `xml/namespaces/header.rs` | Bounded root scanning avoids a redundant full-body pass. Header eligibility is not proof of application origin or zero bloat. |

## Verification and repeatable review

The core suite, focused targeted archive/retention tests and read-only retained-
report contract audit were executed during this review. Broader archive study
tests also passed (188 passed, two ignored before the final additional missing-
image test); the targeted comparison suite passed (62 passed, seven ignored).
The final focused archive/targeted selection passed eight tests with its read-only
artifact audit excluded from normal runs; that audit was executed explicitly and
accepted the 84 retained reports. Counts reflect the commands actually run, not
a claim that every ignored experiment or native corpus case was executed again.

Useful commands (PowerShell environment example):

```powershell
cargo test -p excel-shrink-core --lib
cargo test -p excel-shrink-cli --bin excel-shrink-cli regression::archive::study
cargo test -p excel-shrink-cli --bin excel-shrink-cli regression::targeted_comparison
$env:TARGETED_REPORT_REVIEW_ROOT = 'V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\tdg-20261102'
cargo test -p excel-shrink-cli --bin excel-shrink-cli retained_completed_reports_satisfy_the_contract -- --ignored --nocapture
```

For a source/specification review, take each S/V/E rule and ask: where is its
decision made, which inputs does it use, what invalidates its proof, what evidence
survives, and which independent counterexample exercises it? Treat mismatches as
findings with severity and an acceptance case, not editorial wording to conceal.

Reviewed critical paths include CLI profile resolution, namespace admission,
cell/row/column cleanup and inheritance, deferred references/shared formulas,
workbook-name cleanup, snapshot/content selection, CF dependency selection,
height runs, staged pairs/backfill, capture transport, final classification,
retention, ownership/cleanup and resume filtering. This is not a line-by-line audit
of every legacy experiment, UI handler, platform API or third-party dependency.
The unrelated uncommitted tall-sheet refactor present in the checkout was not
modified or claimed as part of these corrections.
