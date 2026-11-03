# Targeted visual validation behavior

The default strategy is `default-inplace-rebuild-v1`, with capture policy
`targeted-content-grid-v3`. It compares Original with Default (SD), before and
after explicitly rebuilding **the same SD instance**. Original is not explicitly
rebuilt. Native Check Performance is optional and disabled by default.

## States and order

1. Verify raw Original/Default input identity and inspect XML. Byte-identical
   Default input may omit SD and SDR, with its SHA-256 identity proof retained.
2. Open Original (O) and, when required, Default (SD), then ordinary-save both.
   Keep O open without explicitly rebuilding it throughout the comparison.
3. When explicitly requested, generate or restore Check Performance (CP) from
   saved Original. CP has no explicit rebuild in this visual strategy.
4. Select XML/layout-derived regions and charts. Capture and publish O versus
   SD, plus O versus CP when enabled. Retain those images and comparison results.
5. Transfer the existing SD resident session to the SDR reporting lane. Perform
   fullRebuild and save in that same Excel process. There is no extra workbook
   opening or independent rebuild copy. Keep the saved SD pre-state on disk as
   an XML control, not as an open instance.
6. Inspect SD's rebuild effects and O versus SDR. Capture selected regions in
   live O and SDR, and compare O versus SDR. CP can supply late regions while
   it remains open. Close all owned instances with confirmed process exit.

A normal differing-input comparison opens two Excel instances. Enabling CP adds
one independent instance. The SD/SDR PIDs must match. OR and CPR are explicitly
`disabled`; SDR is `observed` with method `inplace-rebuild`. SD always receives
its rebuild when it is observed, including static workbooks.

## Evidence and limitations

The required pairs are O/SD and O/SDR, plus O/CP when requested. There is no
SD/SDR, O/OR or both-rebuilt comparison, and no automatic volatility diagnosis.

The pre-rebuild O/SD comparison freezes its own selected regions before SD is
mutated. New regions can first appear after the SD rebuild, and capture geometry
can repartition. Each pair therefore retains `selected_region_count` and its
actual findings; the SD-before comparison does not claim those late regions.
Post-rebuild O/SDR coverage includes the late selection. The consumer rejects
missing pairs, missing phase-specific findings, unconfirmed CP outcomes and
an SDR process identity that differs from SD. It still accepts historical
version-three reports through their historical execution contract.

The outcome remains `completed_with_limitations`. Selected regions are clipped
to content-bearing cell bounds with margin; formatting-only exterior regions
are excluded. All inventoried charts are checked. Zero-area observations must
be explicit and cannot manufacture pixel equality. Resource, calculation and
transport failures remain failures. XML-selected coverage does not certify
whole-workbook equality.

Excel can calculate during opening even when manual calculation is requested.
O versus SDR retains the effects of an SD rebuild against Original without an
explicit rebuild. A difference is not automatically attributed to Shrink or to
volatile formulas. Investigate those causes separately.

## Commands and reuse

`regression targeted-compare --original O.xlsx --default SD.xlsx --output result`
uses the default no-CP strategy. Add `--check-performance` to enable CP. The old
`--skip-check-performance` remains accepted; the two flags conflict.

For a corpus, use `regression study --targeted-comparison --resident-capture`
and the usual corpus/output arguments. CP requires `--targeted-check-performance`.
The PowerShell launcher similarly accepts `-CheckPerformance`. The corpus launcher config uses `targeted_check_performance: true` for explicit opt-in.

Corpus result receipts include the execution strategy. Resume requires that
strategy and the requested CP setting, so historical six-state results are not
silently labelled as observations from this strategy. Preparation ZIP reuse is
independent of the visual execution strategy.

The optional `--prepare-artifacts-only` command remains a separate generation
path. It can prepare saved/rebuilt controls without rendering, and does not
claim a completed visual comparison. Its Original/CP rebuild artifacts do not
make those states part of the default visual strategy.

### Persistent Check Performance artifacts

`check-performance-artifacts-v1` caches generation artifacts, not visual verdicts.
Corpus workers use `.tmp/regression-cache/check-performance-v1` in the repository,
outside disposable worker caches and individual run directories. Override it with
`EXCEL_SHRINK_CHECK_PERFORMANCE_CACHE_DIR`. Standalone targeted comparisons use
the same subdirectory of the ordinary regression-cache root unless overridden.

The key binds exact raw Original SHA-256, the current saved Original's package
fingerprint, Excel executable SHA-256/version, machine/user, cultures, OS version,
controlled calculation/window mode, and a generation-recipe fingerprint. The
recipe hashes the native host scripts/helpers and Office.js sources, not the
shrinker executable or Default output. Changes to Rust generation semantics must
bump the generation policy. A shrinker-only update can therefore reuse CP.

The saved-package fingerprint streams actual uncompressed member contents and
retains their names. ZIP compression/order/timestamps do not count. All worksheet
XML, formula caches, styles, strings, charts, drawings and unknown parts count.
Only `docProps/core.xml`, `docProps/app.xml`, and narrowly recognized static
workbook absolute-path/revision tags are normalized. Unknown metadata is retained.
Changed saved Original contents, including volatile or externally updated values,
cause a cache miss rather than reusing an incompatible generation.

An entry retains the saved Original generation control, CP before optimization,
CP after optimization/before rebuild, and, when generated, the before/after full
rebuild copies. Rebuild reuse additionally requires the current CP parent's
package fingerprint to match the recorded rebuild parent. Otherwise rebuild CP
normally. A valid pre-rebuild entry does not imply a rebuild artifact exists.
Historical rebuild reuse also requires recognized deterministic dependencies.
Ordinary references/arithmetic and an explicit set of pure built-ins are supported.
Volatile calls (`NOW`, `TODAY`, `RAND`, etc.), external/structured references,
names, unknown functions/features, iterative calculation and unsupported package
parts retain a fresh rebuild. Optimized pre-rebuild CP reuse is independent of
this restriction. The skipped-rebuild guard remains a separate decision.

Native confirmation must be positive. Retain file sizes/SHA-256, creation time,
producer executable SHA-256, environment, and compact existing-launcher
measurements for CP and any cached rebuild. Preserve readiness/startup method,
startup time, memory samples and observed peaks; null means unavailable. Peaks
are observed through the stated sample, not promised full-lifetime maxima.
Later samples before closing update the generation's observations. No extra
Excel open is introduced to measure them.

Files are immutable cache-owned blobs; Excel opens verified copies in the attempt
directory. Publish only after copies are flushed and validated, with one atomic
manifest replacement. Concurrent writers can select different complete generations
but cannot interleave their files. Partial manifests, missing artifacts,
unsupported identities, checksum failures and unsafe object paths do not become
hits. Report cache failures explicitly and perform fresh native generation when
lookup is unusable. A failure during restore fails that attempt rather than
claiming valid evidence. Unreferenced blobs from interrupted publication confer
no authority; cache cleanup must preserve blobs referenced by live manifests.

The cache can be published after successful optimization/rebuild even if later
capture fails. It does not mark the workbook comparison complete. On a hit,
restore native confirmation and generation controls, open/save owned copies,
and discover candidates again. Record current-versus-generation and cached-copy
opening/save controls. Compare all required observed pairs, charts and geometry
under the current strategy. Current reopen differences remain capture candidates.

Reports and Observatory distinguish `generated`, `hit` and unavailable cache
states. `check_performance_cache.historical` preserves original measurements and
their provenance; `current_open_measurements` contains new cache-copy opening
observations. Historical values are never silently presented as fresh measurements.
Cached rebuilds use observed method `cached-rebuild`, with the recalculation step
explicitly skipped, not falsely marked as newly executed. A consumer requires the
cache policy, key, artifact checksums and matching positive native confirmation.

Artifact-only preparation may publish these controls before capture selection.
Its `artifacts_prepared` result leaves visual validation pending. Ordinary-save
and OR/SDR controls use the `excel-control-artifacts-v1` cache policy, with raw
input checksum, saved-parent package fingerprint and native environment binding.
Restoration verifies content checksums. Rebuild dependency restrictions also
apply to these controls. Reopening a restored control still requires current
opening/saving audits in a visual comparison.

The optional Check Performance Save As controls retain the saved baseline,
Optimize All plus Save As output, and the same-session full rebuild plus Save
output. These supplement the independently reopened CPR control. Preparation
records operation timings and process memory samples separately from visual
evidence. See [Prepared Excel controls](../visual-validation/prepared-excel-artifacts.md).

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
| States and order | `targeted_comparison/staged.rs`, `workbooks.rs`, `check_performance.rs` |
| Historical rebuild eligibility | `targeted_comparison/execution_plan.rs`, `static_workbook_metadata.rs`; consumer: `archive/study/prepared/targeted/execution_contract.rs` |
| Check Performance reuse | `targeted_comparison/performance_cache.rs`, `performance_reuse.rs`; native provenance: `office-capture/TargetedPerformanceCache.psm1` |
| V5 | `targeted_comparison/xml_changes.rs`, `cell_stream.rs`, `snapshot_cache.rs`, `change_evidence.rs` |
| V6 | `targeted_comparison/candidates.rs`, `content_bounds.rs`, `evidence.rs` |
| V7 | `targeted_comparison/conditional_formatting/`, `cf_inventory/` |
| V8 | `targeted_comparison/heights.rs`, `staged.rs`; Office.js `office-capture/web/targeted-diagnosis.js` |
| V9 | `targeted_comparison/host.rs`, `image_comparison.rs`; `office-capture/TargetedResident.psm1`; `src/capture_host/resident_diagnosis_image.rs` |
| V10 | `targeted_comparison/charts.rs`; `office-capture/TargetedChartSheets.psm1` |
| V11 | `archive/study/prepared/targeted.rs`, `prepared/targeted/contract.rs` |

Office-capture paths are relative to `excel-shrink-cli/`, not the regression folder.
