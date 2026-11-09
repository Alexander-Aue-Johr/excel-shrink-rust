# Targeted visual validation behavior

## Formula explanations in the comparison viewer

Live and exported viewers show formula dependency explanations only when retained
pixel measurements for the selected capture report `changed_pixels > 0`.
An equal image, a geometry-only difference, or unavailable pixel measurements
does not open the formula inspector or request its live diagnostic endpoint.
Formula dependencies never independently select an entry for the differences
view. Retained diagnostic evidence remains available in the report artifacts.
One changed pixel is sufficient to offer an explanation. The explanation is a
possible dependency path and does not establish the cause of every changed pixel.

The fast corpus strategy is `paired-inplace-rebuild-v1`, with capture policy
`targeted-content-grid-v10`. It compares rebuilt Original (OR) with rebuilt Default
(SDR). Each workbook reuses its existing Excel process. Native Check Performance
is optional; when requested its rebuilt state (CPR) is compared with OR.

## States and order

1. Record exact input identities and inspect raw Original/Default XML. Identical
   inputs may omit the independent Default process with their identity proof.
2. Open Original and Default on owned private desktops with manual calculation
   and forced opening/save recalculation disabled. Ordinary-save their working
   copies and retain raw-to-saved XML controls.
3. When requested, optimize an independent saved Original copy with Check
   Performance. Retain its pre/post-optimization and ordinary-save XML controls.
4. Transfer O to OR, SD to SDR and optional CP to CPR. Full-rebuild and save each
   in the same resident process; retain the saved pre-state as an immutable file.
5. Discover candidate regions from the rebuilt comparison partners, their
   dependencies and effective row heights. Retain raw/save/rebuild changes for
   attribution and the union of relevant input bounds for clipping coverage.
6. Capture selected regions and charts selected by the configured chart mode in
   the rebuilt states. Compare OR/SDR
   and optional OR/CPR. Close every owned instance with confirmed process exit.

The minimum differing-input run uses two resident Excel processes. Check
Performance adds one. Each source/target PID must match. Pre-rebuild images are
omitted for throughput. XML controls expose transitions, while exclusive causal
attribution, volatile-rendering diagnosis and pre-rebuild visual evidence require
follow-up. Old reports retain their original execution strategy and coverage.
Current resume requires the current capture policy and execution strategy.

Original is explicitly rebuilt in this strategy. Both sides therefore receive
Excel's full rebuild, including clearing stale-calculation visual indicators.
A failure or timeout on either side prevents a completed comparison. Equal input
bytes do not bypass an independently requested Check Performance comparison.

## Visual acceptance policy

Owner clarification, 2026-11-04. The preservation target is the human-visible
presentation within the relevant data/visual area. Rare color/brightness pixel
variations that were imperceptible to human inspection have been observed,
especially on drawings/charts. Identified cases are accepted rendering variation
and excluded from Shrink's visual regression requirement. Flaky Excel rendering
is the suspected explanation; that attribution remains a hypothesis.

Retain the raw pixel/geometry observation and the reviewed acceptance reason.

Policy v10 compares captures with matching measured capture/content origins only
within their common top-left-aligned pixel rectangle. Right/bottom overhangs,
including colored fills and borders, are outside this comparison scope.
`capture_extent_ignored` records that scope. Raw geometries and both images are
retained even when the common pixels are equal. Any overlapping changed pixel
remains a difference. Missing measurements, shifted origins and chart captures
retain their conservative verdicts. Existing v9 reports retain their raw verdicts
until an explicit retrospective classification is performed.

The stronger historical white-margin proof remains separately recorded by
`blank_margin_ignored` and requires all nonblank pixels to match.

The required data/visual boundary includes values and formulas, plus empty cells
with visible effective fill or borders, including inherited or conditional styles.
White fill can be visible by suppressing gridlines. Use the union across relevant
original/output states so removed values or visible formatting remain covered.
Charts/drawings retain independent visual coverage.

Below the last relevant row and to the right of the last relevant column,
otherwise empty trailing rows/columns may change height/width. Such exterior
geometry is allowed. A safety row/column around a capture is inspection context,
not an extension of the preservation requirement. Changes confined there must not
become product regression verdicts. Preserve actual overflow or objects extending
into that context. Row/column changes above or left of relevant content can move
it and remain significant.

The existing content-envelope implementation described in V6 does not yet fully
implement this boundary. It excludes pure fill/border cells and includes a margin.
See [R7](review-findings.md#r7-owner-clarifications-and-remaining-visual-boundary-gap)
for inspected code and the newer branch's partial blank-edge correction.

## Evidence and limitations

The required pairs are OR/SDR and optional OR/CPR, with explicit region, chart,
geometry and same-process rebuild evidence. Selection does not certify the whole
workbook. Raw and saved XML controls remain available for follow-up.

Historical `default-inplace-rebuild-v1` compared O/SD and O/SDR without an Original
rebuild. Its phase-specific evidence and limitations remain attached to those
reports. Those reports cannot satisfy this paired-rebuild strategy.

## Capture inactivity and prepared-workbook priority

In the targeted Control Room workflow, the job-timeout setting is a **capture
inactivity interval**, not a total workbook wall-clock limit. Every successfully
persisted, validated nonempty cell/chart image advances the monotonic
`capture_success_sequence`. Validated geometry results and completed capture jobs,
including verified zero-area results, also advance `capture_progress_sequence`.
Both the private-desktop launcher and independent parent watchdog reset inactivity
when either monotonic receipt advances. Heartbeats, phase changes, requests without
validated responses, repeated receipts and counter resets do not extend the interval.
This allows adaptive geometry planning to finish before its first image without
mistaking completed range measurements for a deadlock. Individual command deadlines
remain independent bounds. The parent allows launcher cleanup grace.

Before any verified work, or during a later phase without these receipts (including
rebuild or comparison), the same inactivity interval applies. The recovery configuration uses
120 seconds. This may exclude legitimate capture-free work taking longer than
that; it is a timeout, never an equality or Shrink-defect verdict. The guard
publishes failure evidence and releases the owned Excel job so other groups
can continue. Individual command deadlines remain independent upper bounds.

After a launcher timeout, process liveness is checked against the recorded PID
and creation time. A reused PID means the recorded process has exited; missing
creation-time evidence or an inaccessible process remains unknown. A timeout
receipt with live or unknown processes is `cleanup_unconfirmed`, not confirmed
cleanup. The dashboard refreshes frozen observations for both that status and
historical `failed` timeout receipts, without rewriting their durable reports or
original failure timestamps. An observed process exit is not a fabricated
workbook-success or capture-progress event.


An owned native repair dialog is recognized by its English/German content
prompt. It produces `opening-dialog.json` and a clear startup failure without
accepting a repair or changing the source input. Unknown dialogs are not
clicked automatically.

Disposable capture copies disable an explicitly enabled
`workbookPr.checkCompatibility` flag during their existing view preparation.
This is the automatic check for earlier Excel versions, not a repair or a
file-format conversion. Missing/false settings are left alone; formulas, values,
date system and object-visibility settings are preserved. Source inputs and
prepared Shrink variants keep their original setting. Other blocker detection
remains enabled; no repair/data-loss prompt is automatically accepted.
The generation-recipe fingerprint includes `CaptureWorkbook.cs`, so saved-control
and Check Performance artifacts prepared with different setup rules are not
silently reused under the new recipe.

Capture preparation also sets an explicitly enabled
`fileSharing.readOnlyRecommended` flag to `0` in the disposable copy. This lets
the resident save and rebuild without the author's optional read-only opening
recommendation. Tests check true/false/absent flags and verify reservation
passwords and sharing attribution. This staging operation applies to both
Original and Default observations.
See Microsoft's [CheckCompatibility property](https://learn.microsoft.com/en-us/office/vba/api/excel.workbook.checkcompatibility)
and [Open XML attribute mapping](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.workbookproperties.checkcompatibility?view=openxml-3.0.1).

`tools/order-prepared-by-reduction.ps1` creates an explicit `--workbook-list`
and a CSV from ZIP-directory sizes. The score is `(Original XLSX - Default XLSX)
/ Original XLSX`, descending; ties use saved bytes, then corpus identity. It
reuses prepared outputs, does not shrink again and preserves original size-part
membership. The corpus launcher accepts the resulting path as `workbook_list`.
Regenerate the list when prepared outputs change.
## Commands and reuse

`regression targeted-compare --original O.xlsx --default SD.xlsx --output result`
uses the default no-CP strategy. Add `--check-performance` to enable CP. The old
`--skip-check-performance` remains accepted; the two flags conflict.

For a corpus, use `regression study --targeted-comparison --resident-capture`
and the usual corpus/output arguments. CP requires `--targeted-check-performance`.
The PowerShell launcher similarly accepts `-CheckPerformance`. The corpus launcher config uses `targeted_check_performance: true` for explicit opt-in.

Corpus result receipts include the execution strategy. Resume requires a compatible
capture policy and that strategy. A completed comparison including CP also satisfies
a Default-only request; the reverse requires new CP evidence. Historical six-state
results retain their own strategy and are not relabelled as this strategy.
Preparation ZIP reuse is independent of the visual execution strategy.

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

Process cards retain phase wall times and aggregate host request latency by lane,
operation and sheet. The browser separately measures capture geometry, image
render synchronization and binary transfer, including PNG byte preparation.
Parallel request and phase durations may overlap and must not be added into an
exclusive total. Aggregation retains one entry per operation/sheet rather than
every request. Old results without these observations display unavailable data.

Opening and start-RAM comparisons use the existing resident's
`launch-to-addin-ready-v1` observation. They never open a separate benchmark
instance. Start RAM is the working set sampled at readiness, not a lifetime peak.
In-place rebuild cards retain the source process's single opening observation.
Relative bar colors describe that measurement against its partner and do not
change validation verdicts.

Changed cell locations, changed row metadata, column/default changes, conditional
dependencies and measured row-height changes feed candidate selection. Chart
captures are independent of the cell-envelope policy.

The implemented envelope is the bounding rectangle of content-bearing cells and
potentially visible fill/border/conditional-formatting extents, unioned across raw,
saved and rebuilt states. A value deleted by Shrink or Check
Performance therefore remains in the union. Add one row and one column of margin
at each available edge, bounded by Excel's grid.

A full-width, uniformly applied solid column fill without borders is an
unbounded background. It does not extend the envelope to the worksheet limits.
For this case the envelope retains every serialized cell, including explicit
blank/no-fill overrides, the content and merge extent, and an A1 background sample.
Row styles with the same solid fill and no visible borders preserve this proof,
including font and alignment differences on those rows. Explicit cells still
remain in the envelope. Other row-style overrides, unknown/overlapping column definitions, patterned fills,
unresolved conditional formatting, objects and unsupported worksheet features disable this
proof and retain the conservative extent. Changed column formats and effective
row heights inside the retained envelope still produce capture candidates.

An additional inherited-background rule applies when worksheet views have
supported gridline settings, inherited row/column formats and the default style
have no visible borders, and each visible inherited fill is a supported solid
fill. It requires resolved column definitions, supported worksheet markup and
no conditional-format effect on unserialized blank tails. The envelope retains every serialized cell,
content/merge/bounded-drawing extent, row transition and finite column interval
edge. It includes a following row and a column in the final interval as background
samples. A column interval ending at XFD has no additional finite right edge.
Beyond these transitions the empty background repeats. Different solid color
references are allowed because their pixels are still compared within the
retained extent, without assuming that the colors are equal.

Coverage records expose `background_tail_sampling=true` when any included state
uses this rule. Pixels outside that extent were not captured. This records
representative background coverage and must not be presented as exhaustive
whole-grid pixel coverage. Gridline visibility is sheet-wide and adds no finite
transition after the last recorded row or column layout transition. Visible,
hidden and default gridline settings are supported. Missing views, unknown
gridline settings, unresolved CF, inherited borders, patterned
fills, unknown styles or unsupported markup retain the previous fallback.

A standard CF catalogue consisting entirely of `cellIs equal` rules with
nonempty literal text operands cannot format unserialized blank cells. Such a
catalogue does not extend the content envelope to its whole application area.
Every serialized cell and formula remains covered by ordinary selection. The
proof requires supported namespaces and rule content, resolved column metadata
and supported worksheet features. Coverage records expose
`cf_blank_tail_excluded=true` when at least one included state used this proof.
Another state's fallback can still extend the union. Numeric operands, empty text, other operators,
dynamic expressions, extensions and unknown objects retain the extent fallback.
This also permits the existing inherited-background proofs when CF is inactive
on blank tails. It does not assert equality between the rule outcomes on actual
cell contents.

Intersect candidate neighborhoods with that envelope and put them on the shared
internal grid of **128 rows by 16 columns**. Deduplicate grid regions and clip
boundary tiles to the envelope. This is only a candidate index. Under capture
partition `visible-geometry-v2`, merge adjacent tiles horizontally, then vertically
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
of certifying equality. Measured size excess determines a proportional cell-coordinate subdivision.
Each expansion creates at most 128 children, each independently measured in every
live state. This bounds temporary planning work, not final coverage. Uneven
geometry can trigger further subdivision. Zero-area leaves
remain explicit and generate no image. A single indivisible oversized cell fails
explicitly. Every state uses the same cell partition. Later candidates or rebuild
geometry can repartition; retained earlier states backfill all new leaves before
final comparison. Geometry is cached only within one immutable lane/range.

The envelope is a rectangle, not the exact set of nonempty cells: large internal
empty areas can remain within it. Visible fill/border cells, inherited row/column
formats and conditional-formatting application areas extend the envelope even
without values. Unknown style indices conservatively extend it. Contiguous center-across-selection
spans seeded by a value/formula extend it through their formatted blank cells. Font-only empty
cells do not. Changes outside this expanded envelope are counted as excluded,
never pixel-confirmed equal. Worksheet styles are resolved once per snapshot,
and the existing streaming worksheet pass accumulates the additional bounds.

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

Removing a cell with no value/formula/inline-string payload and no nonnumeric
type does not change its blank CF input. Its visual/style changes remain capture
candidates. Explicit nonnumeric types and payload-bearing cells remain potential
input changes. This distinction avoids turning blank-cell cleanup into a
whole-area dependency change for duplicate rules, scales or data bars.

`x14ac:dyDescent` is recorded as row-local layout metadata. A changed or removed
row still selects its affected CF intersections and height checks, without
invalidating the unchanged rule's entire application area. Other unfamiliar row
metadata retains the global conservative fallback.

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
shared style context check rows from the sheet start through the union of
content-bearing envelopes, minus identical fixed rows. The trailing context row
used by captures does not create a height dependency below actual content.
Empty rows above content are included because they affect capture positions.
The tail outside content and potentially visible formatting is excluded, not certified equal;
charts retain independent full-chart observations.

Measure effective height runs through the same Office.js instances. Responses
must cover every requested span exactly, in order, without gaps/overlaps or
invalid heights. Budget exhaustion/incomplete coverage is an error, not equality.

Compare height runs and retain every difference interval. Capture the interval's
start/end and following row, across the content-bearing columns. This is boundary
evidence for a reported height interval, **not a screenshot of every row inside
the interval**. Exterior ranges can still be excluded by the content policy.

Explicit rebuild uses Excel's calculation API and saves the same SD instance.
The resulting XML changes select indirect formula-result candidates. Original
is not explicitly rebuilt in this screening strategy; no Original-rebuild drift
control is claimed. `NOW`, `RAND`, external links and other
environmental behavior prevent assuming every resulting difference is exclusively
caused by Shrink. Rebuild and original-control observations do not restore a lost
raw pre-opening display state.

The current targeted flow uses these selected indirect changes and controls;
it does not unconditionally revert to the old whole-workbook pixel pass whenever
a hidden or whitespace value was removed.

## V9. Captures, transport and comparison

Capture partitioning batches independent ranges from one sheet. The live
`geometry_batch_ranges` API control defaults to 64 and accepts 1..4096.
Each live state loads all four geometry properties for those ranges before one
Office.js `context.sync()`. Oversized parents produce children for later batches.
Every child is still measured in every state that lacks a cached observation.
Responses must contain every requested address in order and finite, nonnegative
geometry. Missing, reordered or invalid observations fail the attempt. Batching
does not change selected cell coverage, image limits or zero-area handling.

Office.js captures the requested region and measured geometry in its workbook
instance. The independent `capture_batch_ranges` API control defaults to four
and accepts 1..64. Both controls are reread before the next package and retained
across run restarts. Submit at most one outstanding package per distinct open
lane; collect every result in request order. Responses must match the active
session and command. A late-added candidate is captured in all retained states
before their instances close. New late candidates cannot be compared with missing
partners and cannot silently overwrite earlier image filenames.

Each image package stays on one sheet and shares a conservative 32-megapixel
decoded-pixel allowance. Large regions therefore use smaller packages, including
single-image packages. Requested coverage is never truncated to satisfy the
package size. One synchronization obtains all range/core geometries, and another
obtains all nonempty image results. Binary uploads require a distinct validated
image index within that command. A failed member fails the package rather than
certifying incomplete evidence. The UI shows the actual range count per package.

Example live control update through `POST /api/control` on the Observatory host:

```json
{"geometry_batch_ranges":128,"capture_batch_ranges":3}
```

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

Consider every inventoried embedded chart and standalone chart sheet independently
of cell candidates. `--chart-capture-mode deterministic` is the default. It skips
an embedded chart only when its chart XML and caches, static package dependencies,
referenced cells and styles, source layout and visibility, supported CF inputs,
and complete live chart geometry are unchanged. A cache alone is insufficient.
Source references are checked against complete worksheet cell streams, including
coordinates outside the normal worksheet capture envelope.

The first supported references are explicit same-workbook A1 cells and finite
rectangles, including quoted sheet names and absolute coordinates. Defined names,
external links, PivotCharts, user-shape dependencies, unknown extensions or package
features, incomplete geometry and ambiguous object names retain captures. The
recognized static extension payloads are chart unique IDs and `dispNaAsBlank`.
Other charts in a workbook may also retain captures when a shared package
dependency cannot be proved unchanged.

Every actual exclusion has `dependencies_unchanged=true`, explicit
`unchanged-chart-dependencies` rendering states and `pixels_compared=false`.
`chart_dependencies` retains per-pair decisions, reasons and reference counts.
The result contract requires a matching dependency receipt and identical measured
geometry. A state shared by multiple pairs skips only charts proved unchanged
for every pair. The Observatory reports dependency exclusions separately.

`--chart-capture-mode conservative` exports all charts and can be supplied to
both `regression targeted-compare` and `regression study`. The private-desktop
wrapper accepts `-ChartCaptureMode`. The corpus launcher configuration accepts
`settings.chart_capture_mode`. The setting is recorded in run provenance and
reports. Existing full-chart results remain reusable in deterministic mode.
An explicit conservative request rechecks records produced in deterministic mode.

Charts requiring images use Office.js for embedded charts.
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

## Default-only screening and resume

A Default-only run retains Original and Default, compares their selected regions,
then rebuilds the same Default instance and compares it against the retained
Original. The process cards retain the Default card while its label advances
from Before rebuild to After rebuild. Omitted states remain in the workflow table.

Completed results from the same execution strategy that additionally checked
Check Performance satisfy a subsequent Default-only request. Their evidence and
provenance remain intact. A Default-only result cannot satisfy a later request
that includes Check Performance. Missing policy or execution metadata still
prevents reuse.

### Embedded chart export fallback

When Office.js enumerates an embedded chart but its export fails with `ItemNotFound`, the launcher attempts PNG export through that resident's PID-validated Excel application. It does not open another Excel instance. The previous active worksheet and visibility are restored without saving the temporary rendering state. Other Office.js errors and failed native exports remain failures. Reports record `export_method: native-existing-process`; zero-area charts remain explicit non-rendering observations.

### Informational log-chart warning during save

An owned modal `NUIDialog` is inspected through its native accessibility tree only after the PID-validated dialog enumeration. The exact English message about negative/zero values on logarithmic charts, with one enabled OK action and no other message text, may be acknowledged. The warning text, process identity, dialog handle and action are retained in `acknowledged-chart-warning.json`. Unknown messages, repair prompts and multiple-choice decisions remain untouched. Native accessibility operations share the outer operation deadline; a blocked provider cannot keep the comparison alive indefinitely.

### Uniform row-height measurement

A numeric Office.js `RangeFormat.rowHeight` establishes uniformity; mixed heights return null. Excel may round that property differently from measured range geometry. Uniform visible spans therefore use measured `Range.height / rowCount`, retaining exact hidden-row handling and recursive measurement for mixed spans. No pixel-comparison tolerance is introduced. API contract reference [RangeFormat.rowHeight](https://learn.microsoft.com/en-us/javascript/api/excel/excel.rangeformat?view=office-js#excel-excel-rangeformat-rowheight-member).

### Preparation disk reserves

Preparation admission also requires at least 8 GiB free on the Windows system
volume, even when the study output and temporary files are on another drive.
The existing output-volume reserve still applies independently. A disk guard
wait retains completed results and lets admitted work finish. Its diagnostic
identifies the constrained volume; reclaim space before resuming the run.

### Exact image storage sharing

Binary targeted uploads retain their command-specific paths and exact encoded bytes. Within each resident, byte-identical immutable uploads share a content-addressed blob through filesystem hard links. Pools are bucketed by 512 dispatched commands to bound link counts. Existing command retries and blobs are checked against the uploaded bytes before reuse. Unsupported links fall back to ordinary copies. Moving a command image into report evidence preserves its bytes and link; removing a pool entry does not invalidate retained evidence. This storage sharing changes neither candidate selection nor pixel comparison, and requires no special reader when results are copied to another machine.

### Chart-sheet viewport normalization

Standalone chart exports enable `SizeWithWindow` while applying the controlled 1000-by-750-point viewport and restore its previous value afterward. Freezing that property before resizing could retain a chart's initial window-dependent size. Native chart-area geometry remains recorded, so unequal exports remain differences.

Completed corpus comparisons remove temporary upload-pool links after both reports and retained image paths are durable. Retained PNGs remain valid independent hard links. Cleanup is restricted to hash-named `.image` files inside owned resident upload pools, and its removed-file count is recorded. Tests verify retained evidence, unrelated files and idempotent cleanup.

### Embedded authentication dialogs

The targeted launcher observes the atomic `capture-regions.jsonl` index before
geometry partitioning. A literal `A1:XFD1048576` region produces an explicit
`unbounded_capture_plan` failure with unverified coverage. Owned processes are
closed through the existing launcher cleanup, and the raw plan remains retained.
This safeguard does not establish equality or classify the workbook as invalid.
It prevents a known whole-grid expansion from monopolizing a corpus worker while
bounded inherited-background/CF coverage remains unresolved. The observer reads
at most 4,096 index entries and rereads only when the index timestamp changes.
Large bounded-column plans, including `A1:P1048576`, remain eligible for the
ordinary geometric partitioner. Explicit independent reference commands retain
their own supplied-area and image-budget contract.

The persistent PowerShell command host receives a normalized Windows script
path, including when capture assets were resolved through canonical filesystem
paths. Windows verbatim prefixes must not reach its `-File` argument because
Windows PowerShell 5 can otherwise lose `PSScriptRoot` before host initialization.

The targeted resident classifies an owned dialog titled `Name Conflict` or
`Namenskonflikt` as `name_conflict`. It follows the same early-failure evidence
and owned-process cleanup path as other known opening blockers. It does not
choose a replacement name or accept the prompt. The standalone conversion and
Check Performance preparation pool uses the same dialog classification rules.

During opening and outstanding Excel commands, the native host inspects visible dialogs belonging to the exact owned Excel PID. This includes modal `BasicEmbeddedBrowser` windows, which do not use Excel's classic dialog classes. Accessible labels from these browsers are read by a disposable helper with a four-second process deadline and a five-second repeat interval. Editable fields and password values are excluded; email addresses in labels are redacted.

An observed Microsoft sign-in request is recorded as `authentication_required`; `AADSTS50020` or an explicit missing-tenant-account message is recorded as `tenant_access_denied`. The session retains `blocking-dialog.json` with the phase, PID, process birth time and observed labels. The existing early-failure cleanup closes the owned processes and the workbook remains an unverified skip. An empty or still-loading embedded browser does not establish an authentication error. A nonblocking external-links security bar likewise does not establish denied access.

The label reader is launched explicitly on the same named private desktop and assigned to a bounded cleanup job. Ordinary PowerShell child startup is insufficient for this accessibility probe. The integration test exercises the complete browser-dispatch path from a private desktop, including repeated-probe throttling.

### Worksheet names

Workbook-plan worksheet names decode XML entities and then SpreadsheetML Xstring escapes in one left-to-right pass before Office.js lookup. For example `_x0009_` denotes a tab, while `_x005F_x0009_` denotes the literal text `_x0009_`. UTF-16 surrogate pairs are joined and invalid pairs fail explicitly. Relationship targets retain ordinary XML attribute decoding. Encoded names remain conservative for CF proofs that require simple literal names. See [Microsoft's Xstring clarification](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/bd0aa042-434a-4ca7-b25f-4e1fd25a954d).

### Embedded chart identity

Each embedded chart inventory retains its zero-based position in the worksheet's chart collection, alongside its name. Captures use `getItemAt` and verify the expected name after synchronization. Repeated names therefore retain distinct observations. Native same-process export translates the index to the one-based ChartObjects collection and performs the same name check. The chart finding records `chart_index`; older records without an index retain their legacy name-based shape. A missing or reordered observation remains an explicit difference or error. See [Microsoft's ChartCollection API](https://learn.microsoft.com/en-us/javascript/api/excel/excel.chartcollection?view=excel-js-preview), which documents that name lookup returns the first duplicate.

### Pending calculation with external workbook links

The launcher counts standard external-link parts from the ZIP directory. For inputs with such parts, a rebuild that remains continuously responsive in Excel's `Pending` state for 15 seconds fails explicitly with `RebuildPendingExternalLinks`. Active `Calculating` time retains the ordinary command deadline. Observations separated by more than two seconds reset the Pending interval, because opening another workbook can block Office.js. No external-link evidence means the ordinary deadline remains in effect. This short diagnostic budget is based on 5,926 retained successful rebuild-and-save operations: median 0.187 s, 99th percentile 0.787 s, maximum 3.652 s. The sample covers command-position-2 operations and may contain repeated workbooks.

The failed workbook stays unverified. Presence of external-link parts does not by itself establish causality. Breaking links changes formulas into cached values, so a comparison after that transformation requires separate interpretation. The regular validation path preserves those formulas; link-breaking experiments operate on disposable copies.


### Historical cleanup observations in the worker list

A failed attempt can retain `cleanup_unconfirmed` after its own telemetry writer
has exited. The Observatory refreshes these historical process observations
using both PID and creation time. Only confirmed exit of every possibly started
lane moves the attempt into failed history. Unknown or live processes remain
unconfirmed; the original timeout/error and durable report are preserved.
The original failure timestamp orders this history; no process-exit timestamp
or success is invented. Retired run IDs explicitly remove old browser cards
when their corrected failure lies outside the bounded recent history. Late
snapshots of a retired attempt cannot reactivate its card. This observation
does not control or reserve scheduler slots.

### Inspecting retained pixel differences

The result list lazily measures retained PNG pairs, with one image-inspection
request active at a time and an on-disk cache keyed by the immutable report and
selected region. These diagnostics do not run Excel, select captures, change a
verdict, or classify rendering noise as a pass.

`changed px` counts unequal aligned RGBA pixels in the overlapping image area.
`max Δ` is the largest absolute channel difference (0–255); `ΣΔ` sums absolute
channel differences. Workbook totals sum comparison-region observations, not
globally unique worksheet pixels. Pixels outside the overlap are reported
separately as unpaired, including their non-white RGBA count. Missing image
pairs are explicitly unavailable rather than measured as identical.

The atelier overlays changed pixels in red and outlines affected blocks. It
shows nearest-neighbor enlarged pairs with a switchable red pulse. Amber bounds
identify unpaired non-white content, which can result from a cropped edge rather
than a workbook change. Up to 64 highest-delta display blocks are retained for
navigation; all overlapping changed pixels remain in the mask. Clicking either
image permits manual inspection beyond these display blocks. Checkerboard in a
crop denotes absent image area. Reduced-motion settings disable the pulse.

`Copy Original file` extracts a fresh inspection copy from the immutable prepared
bundle and places that copy on the Windows file clipboard for Explorer paste.
It neither opens Excel nor exposes a live worker workbook for editing. Copies
use the host's temporary directory (configured on V: for this validation run).
The helper uses Windows PowerShell's installed clipboard support, without a
separate .NET SDK requirement. A failed clipboard operation is shown explicitly.

The atelier has separate previous/next controls for regions within the current
file and differences across all files. Cross-file navigation loads the paginated
differences index independently of the sidebar filter, rejects pages from a
different run, and stops at either boundary. Moving backward into another file
selects its last different region; moving forward selects its first. Enabling
checked regions does not insert identical regions into the overall-differences
navigation. No workbook is opened or recaptured by these controls.

### Formula dependency explanations

After differing cell regions have been compared and Excel has closed, the
optional diagnostic reads the actual saved states before their disposable files
are removed. It records changed saved values and one supported path to RAND,
RANDBETWEEN, RANDARRAY, NOW or TODAY. Direct/absolute A1 references, quoted sheet
names, hidden worksheets, ranges, shared formulas and literal A1 INDIRECT calls
are supported. Computed INDIRECT/OFFSET, external/structured references and
unsupported paths remain unresolved. The tracer does not evaluate Excel formulas.

The atelier renders separate cell cards and arrows below the images. Only the
balanced volatile call is highlighted, using browser UTF-16 offsets. Source
workbooks and live worksheets are never marked or edited. A dependency is not
exclusive causation: an unused IF branch can contain RAND, for example. Unknown
or mixed findings are not automatically moved below other findings or accepted
as rendering noise. The list annotation currently appears after a region's
formula explanation has been inspected; persistent classification/sorting is
separate follow-up work.

Older reports can show dependencies from the prepared Original instead. Their
source cached values are explicitly labeled and never presented as the values
rendered after a historical rebuild. The backend caches this read-only view.

Diagnostic budgets are independent of scientific coverage: at most 100,000
indexed cells/shared strings and 16 MiB of diagnostic text per workbook; 100,000
search operations per region; at most 64 nodes in a displayed path and 20
retained example paths. New reports inspect all changed values while the search
budget lasts, even after the example limit is reached. Retention is bounded to
32 diagnostic regions and 1 MiB per report. Exceeded or unsupported diagnostics
are explicit; no capture candidate or comparison is omitted because of these
budgets. They do not reinstate worksheet-size or capture-count limits.

The native Cluster2 control finds 722 changed saved values and supported volatile
paths for all 722. The source/default input payloads were equal before Excel
recalculation. This evidence is retained separately; raw pixel verdicts remain.

## Cell-format identity during XML selection

Excel may reassign cellXfs and cellStyleXfs indices while saving or rebuilding a
workbook. Selection resolves explicit cell, row and column format indices to the
complete referenced cell XF and its base XF. All formatting properties remain in
the fingerprint. Cells without an explicit style also include the implicit default.
Index-table counts, order and unused named-template labels are not applied visual
properties. Unrelated named templates therefore cannot change every cell's identity.

Cell and base formats resolve font, fill and border indices to their complete
definitions, and custom number-format IDs to their format properties. An omitted
component index still depends on component zero. Unused component definitions do
not change applied formats. Excel named-style revision GUIDs are editing metadata;
they are recognized only with the expected revision namespace binding.
Differential formats, colors, themes and other shared context remain dependencies.
Unknown XF/component properties or extension links retain the complete raw-style
fallback. Empty built-in slicer/timeline default declarations remain in the shared
context without preventing index resolution. Invalid referenced format indices
are explicit errors, not evidence of equality. Different serialization of a
component can still conservatively produce a candidate.

An empty custom number-format definition may use either a self-closing element
or an explicit closing tag with only whitespace in between. Both spellings have
the same identity. The format code and other attributes remain dependencies,
including when a workbook explicitly defines a built-in-range number-format ID.
Unexpected body content does not qualify for this equivalence.

Constant cellIs rules and recognized value-reference formulas have a narrower
rendering context: their comparison does not depend on unused cell-format templates.
Changed applied formats retain their own cell/row/column candidates. The complete CF rule,
differential formatting, shared strings, theme/workbook context and implicit default
must still agree; changed or uncertain cell inputs remain capture candidates. Other
rule families retain the complete applied-format safeguard. Real column-layout or
row-layout changes keep their existing conservative handling.

The CF workbook-property context excludes only `checkCompatibility`, which
controls an automatic compatibility-check dialog rather than rendering. Other
properties, including `date1904` and `showObjects`, remain dependencies.

Only internal comparison fingerprints are normalized. Original, Shrink and retained
Excel-saved files are not rewritten by this analysis. Raw audit transitions remain
available. The capture plan still includes the existing transition union; restricting
that union to the final rebuilt pair is a separate policy decision.

The retained `paleosol_hyland_2013a_b4f9a9e0b89c.xlsx` case demonstrated this
failure mode. Its seven x14 expression rules apply down to row 1,048,576 and refer
to fixed cells on the input sheet. The old rebuilt-pair analysis selected 73,728
CF grid candidates because its global style context changed. Resolving format
identities reduced that pair to 35 cell candidates and excluded all seven rules
with unchanged referenced inputs. With the existing transition union and adaptive
geometry still enabled, the native replay completed in 71.679 seconds: nine image
regions per rebuilt state, zero pixel/geometry/rendering differences, both owned
Excel processes confirmed closed. Selection/transport took 9.467 seconds and image
comparison 1.688 seconds. The previous attempt timed out after 900 seconds during
geometry planning. These are observations of one case, not corpus-wide timings.

For compact offline inspection of another retained pair, set `XML_DIAG_LEFT` and
`XML_DIAG_RIGHT` to the XLSX paths and run
`cargo test -p excel-shrink-cli inspect_retained_state_selection -- --ignored --nocapture`.
This reports candidate counts and bounded CF reasons without opening Excel or
changing a corpus result. XML candidate counts precede adaptive image partitioning.

The `National and Sub-national US Climate Assessment Social Science Incorporation.
_49db76db53b7.xlsx` replay exposed a second case. Excel removed the unused Percent
base template after Shrink removed the single blank percent-formatted cell F360.
The old global context selected 8,403,894 XML candidates, including an almost
full-grid constant cellIs area. Resolving applied base formats and separating the
constant-rule context reduced the rebuilt-pair selection to 25 candidates; two
small duplicate-rule areas retained their complete-context fallback in that replay.
With transition audits and adaptive partitioning still enabled, the native replay
completed in 51.500 seconds: eight image regions per rebuilt state, zero
pixel/geometry/rendering differences, both Excel processes confirmed closed.
Selection/transport took 6.320 seconds and comparison 0.958 seconds. Its previous
attempt was still measuring geometry after more than 40 minutes with no image.
No arbitrary region-count or worksheet-size exclusion was introduced.

Whole-area input proofs now resolve applied cell formats before deciding whether
their inputs changed. Removing an unused font or XF preserves the proof when
the rule catalogue, shared format context, cell inputs and applied formats agree.
A changed nonempty cell format is an uncertain input and selects the dependent
area. Changed inherited row formats also select that area when affected rows
contain value-bearing or explicitly typed inputs inside the rule's range.
Changed formats in empty rows and changed row heights remain local layout
candidates. Formula inputs retain their conservative whole-area dependency.
This applies to the already recognized duplicate, scale, rank and data-bar rules.
Mixed and input-independent rule proofs retain their full format-context check.

### Conditional-format column scope

Conditional-format layout guards compare column properties only through the
rightmost application column, including preceding columns that can move the
area horizontally. Changes exclusively to later columns do not invalidate an
otherwise established input proof. Column styles use semantic format identities.
Unknown attributes, invalid ranges and overlapping column definitions retain
the conservative fallback. Other worksheet layout guards and effective row-height
checks remain in force.

An offline replay of retained rebuilt states from
`Project Summary_SciHub_ASB_d5759637e48f.xlsx` eliminated eight layout-context
fallbacks caused by changed columns BK:BS beyond the affected CF areas. The
combined cell/CF candidate list fell from 15,261 to 7,384 entries, before native
capture planning. Remaining formula-input candidates still require observation.
The earlier native result remains reusable because it checked a superset.

### Bounded display registry

The Observatory returns recent completed attempts from the dedicated
`control-room/targeted-comparisons` registry. Its maintenance task keeps the
latest 200 finished mirrors and checks older entries every five minutes.
Archiving requires a matching run-ID filename, a known terminal status and
confirmed closure for every recorded PID. Unfinished, unknown and unconfirmed
entries remain available for diagnosis.

Older eligible mirrors move unchanged into unique sweep directories under
`control-room/targeted-comparison-history`. Run evidence remains under its
original report directories. Registry maintenance runs independently of HTTP
queries and scientific scheduling. The additional history buffer preserves the
UI's 100 most recent completed attempts while publishers finish concurrently.


### Optional equal-image retention

Corpus launches accept `--retain-equal-images true|false`. The corresponding
launcher setting is `retain_equal_images`. Existing configurations default to
`true`; the Enron run uses `false` to bound retained evidence storage.

With retention disabled, capture selection, native resolution and comparison
remain unchanged. Successfully compared equal images are removed only after
both durable result reports have been published. Receipts retain SHA-256,
encoded length, image dimensions and the positive comparison verdict. Missing,
unperformed or differing comparisons never qualify. Any image shared with a
differing comparison remains full resolution. Equal neighboring tiles are not
retained under this storage policy. The Atelier explicitly labels deliberately
discarded images. Dependency-proven chart exclusions remain distinct from pixel
comparisons and do not acquire an image receipt.

The storage policy was replayed against three native Excel experiment reports
(equal conservative captures, changed chart inputs and dependency-excluded
charts). Result-contract validation before and after retention agreed in each
case. Portable publishing exported no equal images in the equal cases and
retained differing evidence. Deletion occurs after publication, so failure
before publication preserves the original image files.

### Capture package pixel allowance

`capture_batch_megapixels` is an independent live API control, from 32 to 256 MP
per Excel lane and synchronization package. Its default is 32. Range-count and
geometry-batch controls remain independent. Increasing the combined package
allowance never increases the 32 MP / 30,000-pixel individual image limits.
Both host planning and the Office.js command validate the selected package
budget. See the real-workbook experiment in
`docs/visual-validation/experiments/capture-package-pixels`.

### Inherited-layout candidate guards

Policy `targeted-content-grid-v9` explicitly selects inherited row-format and
column-definition changes in the compared states. Cell XML equality alone
cannot exclude these changes. Column definitions are resolved with linear
cursors over Excel's bounded column domain. Only changed columns through the
content edge are selected; preceding changed columns and unknown metadata keep
conservative content coverage. Changed defaults, global rendering context or
unrecognized layout also retain content coverage. Effective live row-height
checks remain independent.

A height difference above the first content row explicitly selects that first
row, because clipping the changed empty rows must not hide displacement of real
content. Replayed Lysimeter rebuilt states retain the earlier 1,061 selected
grid tiles and eleven coalesced ranges after these additional guards. Prior
policy versions remain readable but are not silently promoted to v9 validation.

### Rebuilt-pair capture selection

Save controls and input/rebuild transitions retain their complete compressed
XML-change audit for attribution. They no longer append cell candidates to the
visual plan. The plan is selected from the actual rebuilt comparison partners,
including cell values/formulas, resolved style properties, inherited layout,
conditional-format dependencies, measured effective heights and chart guards.
Original and Default still rebuild independently in their existing processes.
Check Performance, when requested, remains an independent rebuilt partner.

The raw and saved inputs remain available to content-bound discovery, so deleted
real content cannot disappear from the clipping envelope. Unknown inherited
layout keeps conservative content coverage. No region-count, file-size or
cell-count shortcut changes the equality verdict.


### Independent legacy conversion capacity

For XLS selections, `conversion_processes` controls a run-owned pool of persistent
Excel sessions independently of concurrent Shrink stages (`prepare_processes`)
and `shrink_threads`. Conversion capacity may exceed the logical-core count.
The UI offers up to 128 sessions and retains larger configured values. The API
accepts positive 32-bit counts. Missing settings default to one.

Each session starts a native Excel process on its own private desktop, binds COM
to that owned process and keeps a seed workbook open between conversions. Input
workbooks open read-only with link updates, macros, events and AutoRecover
disabled. Each input closes before its converted output is published. Conversion
opening metrics describe this warm-session opening and include the session ID,
Excel PID and number of converted workbooks. They are distinct from subsequent
visual-validation opening measurements.

XLSX conversion explicitly saves with `xlExclusive` access. The new conversion
copy drops legacy shared editing and its revision history, recorded by the optional
`shared_editing_removed` metric. The input remains read-only. Keeping legacy shared
mode had prevented 58 Enron inputs from saving because Excel could not apply their
revision history. Three retained examples saved with exclusive access. Complete
used-range value/formula checks matched after reopening for two examples. The
third preserved every formula and changed one cached value belonging to `TODAY()`
to the current date. This is an observed conversion-time calculation effect,
separate from Shrink and from the later rebuilt comparison.

Idle sessions immediately accept another queued workbook. Increasing capacity
starts additional sessions asynchronously. Decreasing it retires excess sessions
after their current workbook. A failed or stalled session is replaced after its
owned processes are stopped. A disconnected COM server receives one fresh-session
retry, recorded in the conversion metrics. Password, access and content failures
retain their specific failure outcome. PID creation times and retained process handles
protect cleanup from PID reuse. Windows jobs also bound process lifetime to the
coordinator. Failed cleanup remains an error, never a successful conversion.

New sessions require at least 512 MiB available memory, or the configured global
reserve when larger. The check also considers available commit. It is a current
availability check, not a prediction of future workbook memory consumption.

The preparation scheduler admits at most conversion capacity plus Shrink
capacity for XLS selections. Capacity waits are reported separately and credited
against the preparation deadline. Each conversion phase has its own inactivity
deadline. A frozen wait counter cannot extend that deadline. Parent heartbeat
expiry and explicit termination remain effective. Prepared bundles and completed
results retain their existing reuse rules. The pool drains before preparation
finishes and visual comparisons begin.
### Unreferenced format catalogue entries

Layout selection and effective-height selection use shared rendering dependencies
and the resolved default format. Used cell, row and column styles are compared
through their resolved properties. Removing an unused XF from Excel's saved style
catalogue therefore does not select the entire content envelope. Unknown style
structures retain the raw conservative context. Conditional-format and chart
proofs retain their existing, broader catalogue guards.

This matters for sheets with a fill inherited across every column. Their visible
formatting envelope can reach the sheet boundary even when only a few cells hold
values. Local changes still select local evidence. A change to the default format
or an unresolved rendering dependency still requires conservative coverage.
Previously completed comparisons remain usable because this refinement removes
unnecessary candidates without weakening their retained evidence.
### Filename-dependent formula results

`CELL("filename")` can display the temporary workbook path. Separate comparison
copies therefore produce different strings after rebuilding, even when their
formula XML is identical. Preserve the pixel difference and distinguish this
input-environment effect during investigation. It does not justify changing the
shrinker or declaring unrelated differences equal.

The two Enron `3.555790.FLAR4E54UTFJIDD155M3RVLRNOVFKM4VA` examples contain this
formula in `Tab 1!A13`. Their retained 512 by 239 captures differ only in the
filename text at pixel rows 212 through 218. The dependency explainer recognizes
literal `CELL("filename")` requests, including an optional reference argument,
and follows the existing supported references to those cells. It records
`environment_paths` separately from `volatile_paths` and provides UTF-16 call
spans for the shared live/static formula presentation. The list and Atelier
label filename/environment paths separately from random/time paths. An unused
branch can contain either kind, so a path does not prove exclusive causation.

Dynamic information-type expressions and other CELL modes remain unsupported.
Function-like text in string literals or quoted sheet names is not classified
as a call. Raw saved values, pixel evidence and comparison verdicts remain
unchanged. The retained source for Enron case `113d464e38aa934a9c7d` was checked
through the actual diagnostic reader and identifies `Tab 1!A13` as an environment
path, with zero random/time paths. This follow-up is source evidence and does not
replace captured-state values. Older retained diagnostics remain unchanged.
Explicit live inspection supplements an older diagnosis that lacks environment
classification with `source_supplement`, containing a separately labelled source
check. Saved-state counts and values remain in the original diagnosis. Missing
or unsupported source evidence remains explicitly unavailable in the supplement.
Current diagnostics already carrying `environment_paths` use their retained
evidence directly. A negative older diagnosis cannot exclude an environment-dependent
formula.

Supplemented views use a separate `formula-v3` cache keyed to the evidence,
selected region and prepared source provenance. Scratch copies stay within the
study's diagnostics directory and only their exact owned file/directory is
removed. Inspecting an old result does not rewrite its report or start Excel.
The shared presentation keeps source cached values separate from captured-state
values. Static reports can display a retained supplement, but exporting an older
report does not itself run additional source analysis. See the
[isolated API acceptance](../visual-validation/experiments/filename-enrichment-20261108.md).

### Process timings and manual region inspection

Process cards show the measured opening duration and the working set sampled at
readiness, paired with the other workbook's measurement. The primary bar belongs
to the current card. A ratio at least 2 is red, above 1.2 is amber, below 0.8 is
green, and intermediate ratios are neutral. These are relative cost indicators.
Historical attempts without measurements show them as unavailable.

Expandable timing tables retain phase wall times and per-sheet request totals.
Geometry, image rendering and upload have separate measured durations when the
worker reports them. Shared batch timings are not multiplied across images.
Overlapping durations must not be added to estimate total elapsed time.

`Capture regions` loads at most 100 addresses per explicit request. An atomic
JSONL index records the coalesced plan before geometry measurement and the final
measured partition afterward. The index is separate from live status snapshots.
Older completed reports expose retained compared ranges. Failed attempts without
an index do not borrow a previous attempt's region list.

`Inspect pair in Excel` opens read-only prepared Original and Default copies,
selects the worksheet and range, and arranges their windows in the left and right
halves of the monitor's work area. It is available from the region list and the
atelier for cell ranges. These copies are not retained rebuilt states, so cached
formula values can differ from a rebuilt capture. Opening does not recapture or
change comparison results.

One host-owned COM helper reuses two Excel applications. Closing one application
recreates only that side. Closing a workbook reopens only the missing workbook.
Switching groups closes the previous inspection books without saving and opens
new copies in the same applications. The helper disables macros, link updates
and AutoRecover. It does not bypass access or repair prompts. A timed-out helper
and its owned Excel applications are terminated. Host exit also closes them.
Copies and helper files use the configured TEMP directory, which is on V: for
the Enron launch, and are removed when the inspector is disposed.

### Uniform backgrounds with drawing anchors

A solid, uniform whole-sheet background has no finite lower or right edge.
Supported DrawingML objects with fully bounded anchors contribute their finite
extent to the content envelope and allow the existing uniform-background proof.
They remain rebuild blockers. Unknown worksheet markup, unbounded drawings,
rotated objects and unsupported drawing effects retain the conservative extent.
The proof does not authorize dropping drawing comparisons or treating a failed
attempt as equality.

The Enron investigation of group `cade66d204a8e1b3760d` found 69 objects,
including rotated shapes and lines. Its full-sheet planning remains unresolved
by this bounded-anchor improvement. The interrupted attempt requires a retry.
# Startup probe boundaries

The internal `capture-host inject-addin-probe` command creates a new diagnostic
workbook and preserves worksheet payload bytes. Its alternative relationship
location and catalog settings are experiment inputs. They do not select a new
production injection policy. The optional resident-host `probe_access_log`
setting observes only taskpane GET, config GET and ready POST, with counts,
timestamps and status codes. It is disabled by default and excludes query
strings. Readiness requires a successful Office.js workbook identity query.
Workbook visibility alone does not establish add-in readiness. Native startup
timings include process and add-in initialization, distinct from warm COM open
timings. See the dated add-in startup investigation for measured controls.

The workbook-switch experiment optionally captures a small range and compares
RGBA hashes across repeated source visits. A deliberately changed workbook is
the positive difference control. These readiness observations do not establish
that the resident command protocol can retire a completed comparison and safely
reuse its Excel process. Production reuse requires its own generation boundary
and rejection of stale commands and uploads.

The subsequent resident-command probe preserves this protocol boundary by
closing each workbook and replacing its owned HTTP host, with a fresh session
directory and client credentials at the same local origin. Excel remains alive
with a seed workbook. Old-token ready, result and image requests must fail in
the new session. Capture PNG receipts and release resource counters are checked.
This remains a single-worker experiment pending pool and rebuild acceptance.

The experimental C# workbook session accepts one application reference from the
existing owned-window binding. It opens only read-only comparison workbooks with
link updates disabled, verifies native process and workbook identities, and
requires the seed alone after close. Any operational identity or COM failure
faults the session. The caller must discard the process after such a failure.
COM-reference disposal does not replace native process ownership or termination.

The experimental workbook worker wraps this session in a separate process with
serial GUID-tagged UTF-8 JSON-lines commands. The worker owns its Excel process
job and desktop, while its caller owns the worker process job. Any failed command
retires the worker and terminates its Excel job. A caller timeout requires worker
discard and must not trigger an unverified replay of an open command. Command
framing rejects truncated input and frames exceeding 16,384 characters. HTTP
capture sessions and immutable evidence remain separate from workbook control.

The internal Rust workbook-worker adapter assigns the helper to its process job
before sending the bootstrap gate that permits Excel startup. It verifies the
reported native birth identity. Bounded UTF-8 reader/writer channels apply one
deadline to command write and reply. Response GUID and Excel PID must match.
Closing the owned job is followed by bounded cleanup confirmation. A stopped
process with the same numeric PID but a different birth identity does not count
as the original process still running. Unknown liveness remains unconfirmed.
This adapter does not yet alter the corpus scheduler's worker lifecycle.

Native targeted operations resolve the private desktop from their workbook
session. Older sessions fall back to the group's expected-desktop environment
setting. Missing, interactive or malformed names are rejected. Dialog helper
processes receive the same explicit desktop identity for both process launch
and window inspection. A desktop identity alone does not establish that an
external caller can acquire the workbook's COM application. Failed acquisition
remains a failure and does not authorize a global application lookup.

The experimental worker's `rebuild` operation uses its retained C# application
and requires completed calculation state. `save_copy` serializes an XLSX format
51 snapshot without changing the open workbook name. Pending output is unique
and published without replacing an existing file. A save or rebuild failure
retires the worker. Manual calculation and CalculateBeforeSave=false preserve
separate pre-rebuild caches in the deterministic control. Raw snapshots retain
probe add-in metadata. This contract does not claim Save As equivalence or
coverage of external-link and volatile recalculation cases.

### Experimental warm pool lease contract

The internal workbook pool assigns one comparison exclusively to a worker. Pool
and lease credentials are separate. Every owner is identified by PID and birth
time. Active acquire retries reuse their lease. The most recently released token
may repeat release without changing a newer lease, but cannot issue commands.
Capture origins remain reserved while idle and must be free before release.
Confirmed owner exit retires only its worker. Unknown cleanup quarantines it.
Health readiness requires native process liveness and confirmed cleanup. A busy
slot is reported as busy rather than probed through its active command lock.
This service is experimental and is not yet the corpus scheduler execution path.

The experimental worker also exposes `save_as` through its retained application
reference. It requires an open format-51 workbook, an XLSX target and a target
that does not already exist. Excel receives explicit exclusive access mode and
the worker verifies the resulting FullName and nonempty output. A native failure
retires the worker and retains any partial output for diagnosis. This operation
changes the live workbook identity, unlike `save_copy`. Callers must supply a
comparison-owned destination and serialize access to it.

The Save As path passes the no-Excel guards and command transport compilation.
Native acceptance is pending the user's requested reboot. It remains outside
the production corpus path until the pool probe and independent serialized-cache
checks pass. No CELL(filename), volatile-formula or chart equivalence is inferred
from guard tests.

The worker's `open` command remains read-only. `open_working_copy` opens an
explicitly staged copy writable and checks its exact path and ReadOnly=false.
Both disable link updates and reject the seed. The caller owns staging and must
preserve source bytes. Pool trial 04 passed writable-copy, rebuild, Save As and
snapshot checks with unchanged source hashes. This supersedes the pending native
Save As gate above only for the small deterministic formula controls.

For an explicitly configured pool connection, one resident borrows exactly one
worker and creates a comparison-owned capture host on its assigned origin. Its
owned job excludes Excel. Closure confirms the workbook and lease release while
reporting that the native Excel process is retained by the pool. Warm readiness
uses warm-workbook-open-to-addin-ready-v1 rather than the cold-launch measurement
label. Background error checking is confirmed by the retained C# owner.
The normal worksheet capture/rebuild path passed two serial comparisons reusing
both worker identities. Chart fallback and scheduler integration remain pending.

Pooled native chart enumeration/export executes through the retained C# workbook.
It follows the existing normalized chart-sheet viewport and restores temporary
view state. Embedded fallback checks the requested collection index and chart
name, preserves geometry and treats zero area as non-rendering. Any native export
or restoration failure retires the worker. Two complete controlled comparisons
and forced repeated native embedded exports passed with shared worker identities.
Module imports in the JSON host must remain warning-free, enforced by the worker
protocol test. Corpus scheduler ownership remains a separate gate.

### Scheduler-owned pool acceptance on 2026-11-09

EXCEL_SHRINK_WARM_WORKERS=1 enables the pool for manual targeted SheetShrink-only
corpus comparisons. The scheduler starts two native workers per configured group,
passes its private connection explicitly to comparison jobs and supports live
admission changes up to 32 groups. Uninitialized slots reserve their capture
origins without starting Excel. Admitted jobs initialize extra workers on demand.
The pool observes the run's control file once per second and retires idle workers
outside twice the requested group count. A leased or initializing worker remains
protected until its work completes. A zero target drains the already admitted
jobs, then retires their idle workers.

Group admission belongs to the scheduler. Reducing the idle-worker target must
still let an already admitted pair acquire its second lease, including after a
pause request. Initialization reserves its request identity before releasing the
global admission gate, allowing other slots to initialize concurrently. Retried
HTTP admission timeouts retain that identity. Legacy pool settings without a
control-file path preserve their fixed slot count.
Lease release retries HTTP 409 and transport timeouts for at most ten seconds,
with a two-second bound per request. Each retry retains the same lease token.
The server's repeated-release guard acknowledges completion without mutating a
new lease. Authorization failures and other errors remain immediate failures.
Optional Check Performance cannot select this pool path yet.

The pool process waits for a startup gate. Its coordinator assigns a kill-on-close
job before granting the gate, verifies native PID/birth identity and bounds startup.
Loss of the pool stops admission and cancels active comparisons. Closing the owner
job terminates the pool tree. The shared empty-seed writer preserves conversion
behavior. Capture-script paths are normalized for PowerShell 5.1 at the Rust boundary.

A three-workbook run through the real corpus scheduler completed with all records
unchanged and complete, six released resident leases and exactly two reused Excel
identities. After runner exit, both Excel processes and the pool process were
confirmed absent. Warm readiness observations ranged from 0.700 to 0.923 seconds
for these tiny fixtures. Initial attempts failed before comparing because extended
Windows prefixes reached Join-Path in the Excel resolver and capture script. Those
boundary conversions were fixed before the successful run. Eight focused manual
scheduler tests passed, including fixed-capacity admission and prefetch handling.
## Independent reference OLE observations

The explicit native-reference workflow retains OLE metadata even when object
qualification fails. Worksheet, DrawingML identity and VML shape namespaces are
resolved in their lexical scopes. External relationships are recorded without
fetching their targets. Local payload and preview identities retain byte counts
and SHA-256 hashes. VML point frames have explicit absent, parsed or unsupported
states.

The nested `correspondence` observations group worksheet Choice/Fallback records
by shape ID. A `matched` observation requires consistent branch attributes,
known namespaces, one anchored worksheet record, unique DrawingML/VML identities,
matching local preview target and hash, eight equal anchor coordinates and a
parsed VML point frame. Legacy pixel offsets must correspond exactly to the
worksheet EMU offsets. Ambiguity, external targets, foreign namespaces, malformed
or inverted anchors and preview conflicts produce `unresolved` with a reason.

This match concerns the recorded package relationships and anchors. It does not
qualify rendering effects, account for every other object, establish native-frame
agreement or certify pixel equality. The native reference object guard and
production capture policy retain their previous supported-object requirements.

VML observations additionally recognize a closed Office picture-template subset.
It requires the standard type-75 path and formula sequence, known namespace
bindings and attributes, explicit positive point geometry, and complete unique
picture/anchor properties. Shadows on the shape or inherited from its type,
cropping, rotation, external image-source attributes, unknown elements and
missing/repeated properties remain unsupported. `recognized_template` concerns
this VML subset only. OLE activation properties, native-frame agreement and
all-object accounting still require their own checks before coverage can change.

## Capture job list and pinned details

Warm-pool startup verifies that the deployed capture directory has its sibling
`src/regression/archive_processes.ps1` and `ArchiveProcesses.cs` dependencies.
A missing dependency rejects pool startup before Excel is launched or workbook
comparisons are admitted. The error identifies the missing deployment path.

The Observatory retains a compact, vertically scrollable capture-job list.
Initial history is ordered by start time. Newly observed attempts append below
existing entries. Completion updates the entry in place without moving it.
Active entries have an explicit label and colored marker.

Selecting an entry opens its existing detailed workbook card in a modal dialog.
Telemetry continues updating that card through completion. A selected attempt
remains retained when the server retires its registry entry or the ordinary
100-completed-attempt history limit is reached. It then shows its last received
snapshot. Closing the dialog releases its detailed views and signal canvases.
Worker Overview remains available independently.

