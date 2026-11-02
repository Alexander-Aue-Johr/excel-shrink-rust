# Targeted visual validation behavior

This describes `--targeted-comparison` under policy `targeted-content-grid-v2`,
not the earlier full-sheet/hash-sweep workflow. See [execution and evidence](execution-and-evidence.md)
for durable outcomes, retry rules and ownership.

## V1. Objective and explicit scope

The objective is to detect visual/layout differences in XML-selected regions and
all inventoried charts, while retaining controls for ordinary Excel saving and
recalculation. It is not a proof of complete workbook equivalence. The producer
status is deliberately `completed_with_limitations`, even when no difference is
found. The corpus classifies a successful result as `unchanged` or `differences`.

No candidate-count limit permits silently dropping otherwise selected regions.
The old 64-candidate, 64-MiB worksheet and 250,000-cell restrictions do not apply
to this strategy. Resource/time failures are failures, not equality. This does
not imply constant memory: accumulated change vectors, candidate sets, XML layout
metadata and row-height patterns can still become expensive.

The current targeted strategy is separate from legacy tall-sheet edge sampling,
whole-sheet hash sweeps, per-profile whitespace diagnosis and the former second
visual corpus pass. Those implementations remain in the repository, but their
presence does not make them additional stages of the targeted command.

## V2. Named workbook states

Keep input archives immutable and stage disposable working copies. These are
different scientific states, not alternative names for the same file:

| ID | Construction and meaning |
| --- | --- |
| Raw Original | Original input before this workflow opens/saves it |
| Raw Default | Prepared Default shrink output before this workflow opens/saves it |
| O | Original working copy after opening and an ordinary Excel save, before explicit rebuild |
| SD | Default working copy after opening and ordinary save, before explicit rebuild |
| CP-before | Copy of saved O, separately opened and ordinarily saved before Optimize all |
| CP | That copy after native Check Performance and save |
| SDR | Independent copy of saved SD, opened/saved, explicitly rebuilt and saved |
| CPR | Independent copy of CP, opened/saved, explicitly rebuilt and saved |
| OR | Independent copy of saved O, opened/saved, explicitly rebuilt and saved last |

O, SD and CP remain open and unrebuilt while later candidates can still be found.
The original O instance is never itself rebuilt in place. Rebuilding a separate
OR copy last preserves the earlier comparison state and enables late backfill.

“Before rebuild” does **not** mean “never opened or saved by Excel.” No separate
image set proves the raw input's exact pre-save rendering. Excel can recalculate
during opening despite requesting manual calculation. Both restrictions must
remain visible in claims made from these results.

## V3. Required phase order

```text
Audit raw Original versus raw Default
  -> open independent O and SD concurrently
  -> ordinary-save controls; audit raw-versus-saved states
  -> retain/register O and SD
  -> create CP from saved O; establish CP-before; run Check Performance
  -> audit/register CP
  -> inspect available state pairs; select/capture/backfill regions and charts
  -> publish direct comparisons
  -> create and rebuild SDR, then CPR if enabled
  -> inspect pairs and backfill; publish variant-rebuild comparisons
  -> create and rebuild OR last
  -> inspect pairs and backfill all still-live states
  -> close states with confirmed process exit
  -> finish disk-only comparisons and report
  -> retain evidence, publish portable result, then discard disposable XLSX copies
```

Only the independent initial O/SD launches are currently submitted together.
Rebuild-copy preparation is still ordered. Cell captures can run concurrently
across distinct open states; mutation, save/rebuild and native optimization
commands are explicit ordered phases. It would be incorrect to describe the
present implementation as every Excel state opening simultaneously.

No final “unchanged” classification is published from a partial report. Early
comparisons are provisional and can be regenerated when later evidence appears.

## V4. What is compared with what

With Check Performance enabled, require all nine pairs:

| Pair | Purpose |
| --- | --- |
| O vs SD | Direct Default observation after ordinary saves |
| O vs CP | Direct Check Performance observation |
| O vs SDR | Default after its explicit rebuild against unrebuilt Original |
| O vs CPR | Check Performance after its rebuild against unrebuilt Original |
| SD vs SDR | Default's own rebuild/open/save drift |
| CP vs CPR | Check Performance's own rebuild/open/save drift |
| O vs OR | Original's own rebuild/open/save drift |
| OR vs SDR | Both sides rebuilt: Original versus Default |
| OR vs CPR | Both sides rebuilt: Original versus Check Performance |

Without Check Performance, omit its four pairs and three CP-related states;
the five Default/Original pairs remain. Compare the same final selected unique
region set in every required pair. Added/deleted charts are compared by the
union of their inventories, not silently dropped by intersection.

The legacy summary projection is not the authoritative source for all nine
pairs. Use `comparisons` in the targeted report, retaining its separate labels.

CP-before is an XML/size control, not another retained live image lane. There is
no dedicated CP-before-versus-CP pixel pair. O-versus-CP therefore observes the
combined extra opening/saving and optimization path, not a perfectly isolated
visual effect of clicking Optimize all. Similarly, SD-versus-SDR includes copy
reopening/saving as well as explicit rebuild; the XML controls help separate
those transitions, but cannot establish exclusive pixel-level causality.

## V5. XML snapshots, contexts and transitions

Build workbook/sheet relationships and names, then stream worksheet XML into
disk-backed fixed-size cell fingerprints. Validate supported grid coordinates.
Retain layout attributes, column/default metadata and conditional formatting
catalogues separately. The cell stream is not a full decoded value object model.

Fingerprints observe serialized cell content. They are not Excel semantic value
evaluation: formatting/encoding changes can conservatively create candidates.
Both before and after coordinates are included, so a deleted real value remains
discoverable even if its new cell no longer exists.

Shared value/style context includes `styles.xml` and `sharedStrings.xml`. Changes
to shared context prevent assuming the same cell XML means the same input.
Conditional-format context additionally includes relevant workbook/theme/content
dependencies. The shrinker ordinarily leaves these tables unchanged, but Excel
saving or native optimization can change them.

Audit transitions separately:

- raw Original -> saved O;
- raw Original -> raw Default;
- raw Default -> saved SD;
- saved O -> CP-before;
- CP-before -> optimized CP;
- each parent -> its rebuilt copy's ordinary-save baseline;
- each rebuild baseline -> its explicitly rebuilt/saved result.

Each changed location stores before/after fingerprints or absence, sheet,
location, context identities and an overlap flag. A change also observed in the
Original opening/saving control is labeled as overlapping that transition.
Exact fingerprint/location/context matching supports **overlap**, not exclusive
causal attribution. It does not prove a difference is harmless or solely volatile.

The immutable snapshot cache is scoped to one comparison. It keys archive content
by SHA-256, verifies that the file did not change while parsing and never treats
a saved/rebuilt path as immutable just because its filename is unchanged.

## V6. Candidate discovery and the content envelope

Changed cell locations, changed row metadata, column/default changes, conditional
dependencies and measured row-height changes feed candidate selection. Chart
captures are independent of the cell-envelope policy.

The cell-content envelope is the bounding rectangle of content-bearing cells,
unioned across raw, saved and rebuilt states. A value deleted by Shrink or Check
Performance therefore remains in the union. Add one row and one column of margin
at each available edge, bounded by Excel's grid.

Intersect candidate neighborhoods with that envelope and put them on the shared
internal grid of **128 rows by 16 columns**. Deduplicate grid regions and clip
boundary tiles to the envelope. This is only a candidate index. Under capture
partition `visible-geometry-v1`, merge adjacent tiles horizontally, then vertically
where their column extents match, preserving their exact union and its holes.
Measure each merged rectangle through the existing Office.js workers in every
live state before requesting an image. Cell neighborhoods still use the existing
padding helper; overflow can extend farther than that fixed neighborhood.

A measured zero-width or zero-height rectangle retains geometry and an explicit
`zero-area-range` observation, with no PNG and no pixel comparison. If only one
partner is visible, retain its image and report the rendering/geometry transition.
Indirect formula, conditional-formatting and rebuild discovery is unaffected.

Split only rectangles whose estimated visible image exceeds 30,000 pixels on
an edge or 32 million pixels total. Estimate conservatively at two pixels per
point plus rounding margin; this is a planning assumption, not an API promise.
Actual export dimensions are checked too: an unsupported scale fails instead
of certifying equality. Binary search finds a cell boundary near half the visible
height/width in the oversized state. A single indivisible oversized cell fails
explicitly. Every state uses the same cell partition. Later candidates or rebuild
geometry can repartition; retained earlier states backfill all new leaves before
final comparison. Geometry is cached only within one immutable lane/range.

The envelope is a rectangle, not the exact set of nonempty cells: large internal
empty areas can remain within it. Conversely, exterior formatting-only changes
are counted and omitted by deliberate policy. Entirely content-free sheets can
have no cell captures. That is an exclusion, not pixel-confirmed equality.

`content_coverage` records requested, exterior-skipped, clipped and selected
region counts, with the envelope. `selected_grid_tiles` records the internal
index size; `selected_capture_regions` records the final adaptive partition.
Per-lane `skipped_zero_area_regions` distinguishes non-rendering observations
from rendered images. Exterior empty-cell changes can be counted
before allocating per-cell candidate objects. The later grid conversion does
not make earlier in-memory change accumulation free; see review finding R4.

A changed column does not automatically justify a visual “unchanged” claim:
column/default/context changes can require broad effective-height checks. All
selected images remain subject to the content-envelope policy. Changing that
policy requires a new capture-policy identity and review of result reuse.

## V7. Conditional formatting: dependency selection, not rule evaluation

The selector does not ask whether a changed input happens to leave a rule true
or false. It asks which inputs can influence which output areas. If relevant
inputs changed or are uncertain, capture the dependent target area.

First compare rule blocks, layout and shared visual context. Unrecognized
attributes/elements, changed rule/context, unsupported namespaces, unresolved
areas or pivot dependencies retain conservative fallback captures. Fallback
means the affected rule region within the comparison extent, followed by content
clipping; it is not necessarily the whole workbook or even the whole worksheet.

| Recognized class | Dependency footprint / exclusion condition |
| --- | --- |
| `cellIs` with supported literal operands | Target cell itself; unchanged input/context can exclude its contribution |
| Duplicate values, supported color scales and top/bottom/rank rules | Whole application area's inputs; a relevant change/uncertain formula retains that dependent area |
| Supported standard and x14-linked data bars | Known area/threshold dependencies; unsupported extension content falls back |
| Supported expression formulas | Parsed explicit references; map changed sources back to dependent target coordinates |
| Mixed supported blocks | Union dependencies of all members; one unsupported member invalidates that combined proof |
| Literal error/input-independent rules | Only specifically recognized unchanged cases, with context guards |
| Fixed dummy `INDIRECT` template | Exclude only the tested pattern when target absence is established in both workbooks |
| Pivot rules / external or unresolved references | Conservative fallback |

The expression parser supports relative/absolute A1 references, ranges and simple
sheet-qualified references, constants, known operators and a whitelist of functions
with explicit argument dependencies. Examples include `LEN(TRIM(A1))=0`, `ISBLANK`,
`IF`, logical/text tests, arithmetic and supported aggregate functions. It is not
a general formula interpreter. Unknown names, dynamic addressing, unsupported
external syntax or nesting beyond its bound do not establish absence of dependency.

Relative axes translate per target coordinate; `$` axes remain fixed. Changing
one source may affect a rectangle of targets. A formula-bearing input remains
uncertain even when its stored XML is identical; its dependent target cannot be
excluded on a stale cached value alone. Raw before/after rule catalogues are both
considered so removed rules and references are not lost.

The CF inventory preserves original node evidence for later examination. Its
corpus cost classifications are estimates of fallback scope, not elapsed capture
measurements or proof that a rule was visually changed by a particular tool.

## V8. Row heights and recalculation

Stored XML `ht` is not universally identical to Excel's effective row height.
Select relevant spans from cell/row changes; unchanged explicitly fixed rows
can be excluded from automatic-height scanning. Changes to columns, defaults or
shared style context can require the entire automatic-height range up to
1,048,576, minus identical fixed rows. This remains intentionally conservative.

Measure effective height runs through the same Office.js instances. Responses
must cover every requested span exactly, in order, without gaps/overlaps or
invalid heights. Budget exhaustion/incomplete coverage is an error, not equality.

Compare height runs and retain every difference interval. Capture the interval's
start/end and following row, across the content-bearing columns. This is boundary
evidence for a reported height interval, **not a screenshot of every row inside
the interval**. Exterior ranges can still be excluded by the content policy.

Explicit rebuild uses Excel's calculation API and saves a separate working copy.
The resulting XML changes select indirect formula-result candidates. Original's
own rebuild provides a drift control. `NOW`, `RAND`, external links and other
environmental behavior prevent assuming every resulting difference is exclusively
caused by Shrink. Rebuild and original-control observations do not restore a lost
raw pre-opening display state.

The current targeted flow uses these selected indirect changes and controls;
it does not unconditionally revert to the old whole-workbook pixel pass whenever
a hidden or whitespace value was removed.

## V9. Captures, transport and comparison

Office.js captures the requested region and measured geometry in its workbook
instance. For a batch, submit at most one outstanding cell capture per distinct
open lane; collect every result in request order. Responses must match the active
session and command. A late-added candidate is captured in all retained states
before their instances close. New late candidates cannot be compared with missing
partners and cannot silently overwrite earlier image filenames.

Transport image bytes to the resident Rust host, not a large Base64 JSON response.
Office's capture API may initially return Base64 internally; this does not mean
the host upload is Base64. The authenticated upload route validates command/client
identity, actual image format/dimensions and its existing body budget. Oversize,
invalid or missing uploads fail the attempt; no success is synthesized.

Compare full decoded RGBA pixels and dimensions with SHA-256 fingerprints, plus
the separately captured geometry. There is no perceptual tolerance or automatic
“looks close enough.” Reuse validated immutable image fingerprints within the
workflow; different PNG encodings of the same pixels can compare equal. Identical
invalid bytes are not treated as an equal image.

No current client hash-only mode silently discards potentially needed images.
Late rebuild discovery is why unrebuilt workbooks/evidence stay available until
backfill ends. Preview generation and retention follow the full-resolution verdict.

## V10. Charts and other objects

Capture every inventoried embedded chart and standalone chart sheet in every
retained state, regardless of cell candidates. Embedded charts use Office.js.
Standalone chart sheets use a COM export fallback bound to the existing owned
Excel instance and private desktop, not another measurement-only Excel process.

Added/deleted charts produce presence differences. A chart with measured zero
width/height produces explicit non-rendering geometry rather than fabricated
pixels. A rendering-state change is a difference; both non-rendering does not
claim that pixels were compared. Missing exports without a verified zero-area
explanation are errors.

Chart identity includes worksheet, chart name and chart kind. Renaming can
therefore appear as removal/addition rather than only a pixel difference.

Arbitrary non-chart shapes, pictures, VML, text boxes and all possible drawing
layout effects are **not exhaustively modeled by the targeted selector**. Some
legacy rendering experiments support bounded object capture; that is not proof
of complete integration into this workflow. The report retains this limitation.

## V11. Result semantics and acceptance cases

The final report must contain all requested unique pairs and the same unique
region set in each, with explicit Boolean pixel/geometry verdicts and evidence
references. Missing arrays/flags/pairs are errors, not empty successful comparisons.
Chart findings explicitly include presence and rendering flags. Zero selected
regions is valid only as a disclosed scoped result, not whole-sheet certification.

Required tests include: a deleted real cell outside the optimized used range;
exterior formatting-only deletions; a late formula candidate found after rebuild;
a formula input in an otherwise unchanged CF region; absolute/relative/cross-sheet
CF dependencies; incomplete height coverage; added/deleted/zero-area charts;
missing partner captures; invalid images; equal pixels with different encodings;
and a successful-looking but structurally incomplete report.

## Source map

Paths below are relative to `excel-shrink-cli/src/regression/` unless noted.

| Rules | Implementation |
| --- | --- |
| V2-V4 | `targeted_comparison/staged.rs`, `workbooks.rs`, `check_performance.rs` |
| V5 | `targeted_comparison/xml_changes.rs`, `cell_stream.rs`, `snapshot_cache.rs`, `change_evidence.rs` |
| V6 | `targeted_comparison/candidates.rs`, `content_bounds.rs`, `evidence.rs` |
| V7 | `targeted_comparison/conditional_formatting/`, `cf_inventory/` |
| V8 | `targeted_comparison/heights.rs`, `staged.rs`; Office.js `office-capture/web/targeted-diagnosis.js` |
| V9 | `targeted_comparison/host.rs`, `image_comparison.rs`; `office-capture/TargetedResident.psm1`; `src/capture_host/resident_diagnosis_image.rs` |
| V10 | `targeted_comparison/charts.rs`; `office-capture/TargetedChartSheets.psm1` |
| V11 | `archive/study/prepared/targeted.rs`, `prepared/targeted/contract.rs` |

Office-capture paths are relative to `excel-shrink-cli/`, not the regression folder.
