# SheetShrink and DeltaSieve evaluation protocol

Protocol draft frozen on 2026-11-08. Amendments must retain their date and reason.
This protocol specifies prospective experiments. Results require separate measured
receipts. Existing corpus observations form a retrospective development set.

## Preservation and comparison subjects

SheetShrink Default is the production cleanup subject. Preserve meaningful cell
values, formulas, dependencies and human-visible presentation within the
relevant visual area. Include effective visible fills, borders, conditional
formatting and objects when determining that area. Preserve overflow reaching
beyond a populated cell. Empty exterior rows and columns may change geometry.
Protection preservation applies within the relevant visual area according to the
owner's stated contract. Evaluate that contract separately from formula and value
preservation. The hidden-content policy permits removing unreferenced hidden
values under its dependency rules. Record these intentional removals separately
from required-value preservation.

Measure the primary pair after rebuilding and saving both Original and Default
in their respective resident Excel processes. Keep raw, ordinary-saved and rebuilt
XML identities so native Excel transformations can be investigated. XLS inputs
first undergo conversion. Report the conversion stage and failures independently
from cleanup and visual comparison.

Check Performance is an optional independent baseline. Its absence leaves the
SheetShrink preservation experiment valid but cannot support comparative claims
about Check Performance. To attribute its changes, compare a saved control with
the optimized output under matched rebuild and rendering conditions.

## Corpus accounting

Freeze source archive checksums, archive member identities and selection before
each evaluation. Record every selected workbook through conversion, cleanup,
candidate planning, rendering and comparison. Each stage records successful,
excluded, skipped, failed and unfinished counts with reasons. Preserve the actual
denominators. A failed workbook never contributes an equality observation.

Use Data.gov, Destatis and the current Enron archive as separately reported
populations. A later recursively extracted Enron corpus receives a distinct
manifest. Deduplication retains provenance and reports both unique packages and
source occurrences. Freeze results at the paper's reporting cutoff and retain
the unfinished count even if the run continues afterward.

## Independent validation of candidate selection

Use three complementary experiments.

1. Mutations with known expected effects exercise values, formulas, inherited
   styles, row and column geometry, hidden content, merges, overflow, conditional
   dependencies, charts and drawing anchors. Include edits to shared tables that
   preserve referenced IDs and ID renumberings that preserve effective values.
   Confirm expected visible effects with the native renderer independently of
   candidate selection. Record mutations that Excel normalizes away.
2. Select a reproducible, seeded random sample from the frozen corpus before
   observing its visual findings. Stratify by source, package size and structural
   features available before validation. Record the seed, complete eligible frame,
   strata and chosen member identities. Include unchanged and low-reduction files.
3. Supplement with a separately labeled stress set of large sheets, unusual
   dependencies and historically difficult workbooks. Keep this development set
   separate from the random sample when summarizing rates.

For feasible sampled workbooks, render the full relevant visual area independently
of the candidate selector. Use native measured geometry to split oversized images.
Compare the full-reference differences with selected-region coverage. Missing
visible differences are selector failures even if the selected captures match.
Keep full-reference timeouts visible. Do not silently replace a timed-out sample
with an easier workbook. Report coverage by completed workbook and visible area.

## Rendering and attribution

Retain exact pixel observations, geometry, source state and review classification.
Repeated same-input rendering controls estimate variability under the recorded
Excel build, fonts, display scale and renderer configuration. Tiny pixel deltas
receive no universal automatic tolerance. A volatile dependency is evidence for
one causal path and does not account for every changed pixel in a region.

Separate confirmed product regressions, reproduced native rendering variation,
verified exterior-canvas differences, environment-dependent values and unresolved
findings. White trailing canvas can be ignored only under the documented origin,
content-containment and matching nonblank-pixel checks. Keep initial observations
when a later fix or retry changes the verdict.

## Runtime and resource measurements

Measure cleanup wall time, original opening time, cleaned opening time and the
sum of cleanup plus cleaned opening. State the readiness signal and process
isolation used for each opening measurement. Record startup RAM and peak RAM with
their sampling interval. Report single-thread and parallel cleanup separately.
Historical Python implementations have different preservation behavior. Their
runtime comparisons need output-size and correctness differences alongside them.

For DeltaSieve, report planning, geometry, native capture, transfer, image
comparison and persistence time separately where instrumentation permits. Include
capture count and pixels, retained bytes, concurrency and batch configuration.
Compare candidate selection against full reference on the same completed sample.
Use controlled repeated runs for batch and concurrency experiments. A changing
production queue alone cannot establish a causal speedup.

Estimate full-corpus rendering cost from measured geometry and stratified native
capture timings. Row counts alone cannot represent hidden rows, widths, objects
or rendering overhead. Present extrapolations with their assumptions and measured
range. Keep projected cost distinct from completed observations.

## Reporting limits and reproducibility

Publish implementation commits, manifests, producer and renderer identities,
configuration, timeout policies, stage counts and coverage limitations with the
exported findings. Remove machine names, credentials and local absolute paths
from public artifacts while retaining a private provenance mapping. Preserve raw
evidence for unresolved differences.

Successful targeted comparisons support the stated selection methodology on the
tested inputs and environment. Claims of selector coverage require the independent
reference experiments. Finite tests do not establish preservation for every
possible workbook. Release the predefined mutation cases and sampling manifest
so the evaluation can be repeated.
