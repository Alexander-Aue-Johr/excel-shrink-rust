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

### Automatic compatibility check: BOM Aquantis, 2026-11-05

`BOM - 250 kW Aquantis test bench_Status 2014-11-14_b382055a9e33.xlsx`
opened but blocked saving with the Compatibility Checker. Its workbookPr had
`checkCompatibility="1"`. An isolated experiment changed only this attribute
to `0` in Original/Default copies. Excel saved and rebuilt both without a prompt;
the saved copies preserved every inspected nonempty cell value and formula.
There were no charts, drawings or tables before or after saving.

A second experiment retained the unmodified input files and disabled the flag
only in the staged copies through `CaptureWorkbook` preparation. Its native
rebuilt comparison completed: four regions per state, no pixel or geometry
differences. Setup tests cover Transitional/Strict namespaces, optional prefixes,
source immutability, date system, object visibility, idempotency and enabled,
false/absent compatibility flags. Evidence is under
`.tmp/validation-temp/bom-compatibility-proof/native-setup-test/` on V:.
This does not accept unknown prompts or certify all possible save transformations.

The artifact-only replay also exposed stale telemetry after successful cleanup:
`preparation.json` said artifacts_prepared, but telemetry still said running.
Commit `d3b1c0c` sets the terminal preparation status only after owned Excel
cleanup succeeds, with visual validation explicitly pending. The subsequent
native `preparation-result/telemetry.json` confirmed this terminal status and
completion timestamp. This result is not counted as a visual comparison.

### Referenced components: RN17B-RN30 Core, 2026-11-05

`RN17B-RN30 Core_65edc172e0f9.xlsx` has containsBlanks rules extending to
row 1,048,576. Saved Original has 80 fonts and five custom number formats;
saved Default has 79 and four. The removed font was referenced only by empty
cells. Revision GUIDs on named styles also previously disabled format resolution.
Commit `7dd3949` resolves referenced components, recognizes revision metadata
with its namespace guard, and keeps value-reference CF proofs independent of
unused applied formats. Unknown properties/extensions remain conservative.

The retained rebuilt-pair XML diagnostic dropped from 63,361 to 6,764 candidates
(cell candidates 9,553 to 5,686). Of 28 CF decisions, four were excluded and
24 retained partial coverage. A fresh native run, including its normal transition
union, completed in 61.326 seconds with 18 regions per state. Capture selection
and transport took 11.407 seconds; image comparisons took 2.833 seconds. Both
owned Excel processes closed. These single-run times are engineering evidence,
not a controlled throughput benchmark.

Six regions still report pixel and geometry differences, involving blank-region
layout on RareEarths, SingleAnalyteObservation, TraceElements, Volatiles and
WRMajorElements. They remain findings for separate scope/row-height review;
the faster completion does not establish visual equality. Raw reports survive at
`.tmp/validation-temp/rn-core-components-proof/result/` on V:; before/after XML
diagnostics are `rn-core-selection.log` and `rn-core-components-selection-v3.log`
in the same temporary root. The focused targeted suite passed 104 tests, with
ten explicit experiments ignored by that command.

Follow-up: the six RN differences were confirmed as conditional-region layout
regressions. In a disposable counterfactual, restoring only 50 deleted explicit
custom-height row headers eliminated every difference, without restoring cells.
Those heights position the ends of bounded conditional fills below the headers.
Commit `447acba` makes conditional application ranges a row-layout boundary for
range selection and final tail trimming. The metadata scanner handles ordinary
and extension sqref forms; it does not evaluate rule truth.

A fresh shrink with this fix completed the native comparison in 52.769 seconds:
18 regions per state, no pixel or geometry differences, both Excel processes
closed. Evidence is `rn-core-components-proof/fixed-result/` in the same temporary
root. The general boundary also retains existing font/layout representatives
inside conditional scopes: this fixture has 6,273 cells and 664 row headers,
versus the old Default's 5,376 and 496 (Original: 12,782 and 667). Its XLSX grows
from 221,731 to 225,824 bytes. This is broader than the 50-header counterfactual;
it preserves the existing rules for automatically sized rows without assuming
that their fonts can be dropped from other workbooks. It does not keep every
original cell in the protected rows. Further reduction of these representatives
would require a separate height-preservation proof.

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

The read-only cache audit also found `gnipc_829262f4aa5a.xlsx`, where the old
Default omitted unstyled M247 beneath row style 15. Fresh Default retains it.
The native rebuilt comparison completed with one selected region and no pixel
or geometry differences. Evidence is under
`.tmp/validation-temp/gnipc-mask-proof/result/` on V:.

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

### Targeted prepared-cache migration, 2026-11-05

The completed conditional-layout inventory examined 8,622 prepared workbooks;
the unstyled-mask inventory examined 8,623. The 44 bundles changed during the
first inventory received a separate coverage check. A data-validation range
initially reported as a conditional-layout risk was reviewed and excluded;
the corresponding parser-scope regression test preserves that distinction.

The union contained 189 potential cases, not 189 confirmed visual regressions.
Private Default replays found 40 already updated outputs, 12 outputs with
identical uncompressed package parts and 137 different outputs. Equality checked
member names and SHA-256 of every uncompressed member, rejecting duplicate names.
The 12 equivalent outputs retain their existing comparison evidence.

After the runner drained, the 137 different prepared bundles and their records
were backed up with verified hashes before invalidation. Old reports and PNGs
remain available. Each reset record links its previous producer, report and
backup through `artifact_invalidations`. The regular corpus runner regenerates
these bundles; a private replay is not substituted into an old bundle with stale
preparation metadata. The migration manifest is under
`.tmp/validation-temp/backups/before-remaining-layout-20261105-194214/` on V:.

### Whole-column duplicate rule with removed unused formats, 2026-11-05

`permitsummaryissuedfebruary2014_65e8c5127739.xlsx` contains a duplicate-values
rule for B1:B1048576. Its table ends at row 452. Saving the shrunk state removes
one unused font and two XFs, and moves formatting from blank cells into 33 row
headers. The complete format-catalogue guard selected the million-row rule.
The old native attempt planned 2,176 image regions per rebuilt state.

The input-dependent proof now checks the shared rendering context and the
resolved formats of actual input cells. Changed inherited row formats trigger
the whole-area dependency when those rows contain inputs inside the rule's
application area. Empty-row formatting remains a local layout candidate.
Targeted counterexamples cover changed values, uncertain formulas, applied fonts,
number formats, differential formats and inherited row formats with input cells.

Offline selection on the retained rebuilt pair fell from 9,994 to 1,835 grid
candidates. With the complete production transition union, Excel replay selected
two image regions per rebuilt state and reported zero differences. Selection
and transport took 1.420 seconds, image comparison 0.113 seconds. Both owned
Excel processes closed. The replay inputs and report are retained under
`.tmp/validation-temp/permit-aggregate-proof/` on V:. These observations describe
this case and retain the existing conservative coverage for changed inputs.

### Display-registry maintenance, 2026-11-05

The dedicated display registry accumulated more than 6,700 snapshot mirrors.
Every summary read opened the entire registry before retaining recent attempts.
A reversible bootstrap archived 6,527 old terminal mirrors with verified backup
hashes, retaining every one of the 103 entries in the preceding bounded API view.
The backup and per-file manifest are under
`.tmp/validation-temp/backups/dashboard-mirrors-20261105-212116/` on V:.
Legacy mirrors without a source-location field retain their complete original
bytes in that archive. Scientific reports and captures use their existing paths.

After archiving, one summary request took 0.858 seconds. Earlier requests routinely
took several seconds. This is an operational observation during a live corpus run.
Periodic host maintenance now archives eligible mirrors outside a 200-finished-
attempt buffer. The 34 control-room tests pass, including unchanged archived
bytes, recent history, unfinished work, closure uncertainty, malformed metadata
and idempotent repeat sweeps. The running older host received the bootstrap
archive. Automatic sweeps require the newly built host.

### Trailing height context selecting full-width sheets, 2026-11-05

`TwoCylinder_Spacing2.0_MassRatio1.34_Summary_bb043464157e.xlsx` selected 1,027
image regions per rebuilt state. Four sheets each contributed 256 regions because
their first empty context row changed from 20.0625 to 15.75 points. Their actual
content ended immediately above these rows. A full-row border elsewhere in each
sheet made the rectangular content envelope reach XFD, amplifying those exterior
height candidates into full-width captures. The 101 cell-XML candidates alone
did not require this selection.

The existing native report confirms that all measured height differences occurred
in those trailing context rows. It reports zero differing cell regions after
the complete 1,027-region comparison. Height selection now ends at the unpadded
content boundary, while captures keep their surrounding context. The focused
regression test checks both boundaries, and the targeted suite passes 107 tests.
Earlier, more extensive comparison evidence remains reusable.

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
