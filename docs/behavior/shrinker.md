# XLSX shrinking behavior

See [the index](README.md) for status terms and review scope.

## S1. Inputs, outputs and preserved package content

The ordinary CLI accepts XLSX files, or directories of XLSX files. An XLSX is a
ZIP package of XML parts, relationship parts and binary assets. The core does not
open Excel or evaluate formulas. Legacy XLS conversion belongs to the regression
pipeline and is not a capability of the XML shrinker itself.

The shrinker may rewrite worksheet XML, selected workbook defined-name metadata,
and calculation-chain entries for cells removed from processed sheets. It reads
styles, shared strings, relationships and object/formula sources to make decisions.
It does **not** generally deduplicate or renumber the shared-string table, style
table, fonts, images or chart data caches. Existing XF indices are reused.

Unaffected parts are copied; where supported the writer reuses compressed ZIP
entries. Package identity, uncompressed XML identity, compressed file size and
visual equality are different properties. A reduction percentage in compressed
XLSX bytes must not be confused with XML bytes removed by a particular rule.

The CLI honors output-overwrite and selected-sheet options. Selection constrains
which worksheet bodies are cleaned; workbook metadata and reference analysis
remain separate concerns. Input bytes and existing outputs must survive an
eligibility exclusion. Namespace rejection during processing aborts the partial
output instead of publishing a mixed partially processed archive.

## S2. Supported XML representation and eligibility

The fast cleanup parser targets the unprefixed Transitional SpreadsheetML
representation emitted by common Excel files. The default namespace itself is
normal XML; it is not forbidden. An element such as `<row>` in that namespace is
different from an explicitly prefixed `<s:row>`.

Before expensive processing, check `xl/workbook.xml` first, then applicable XML
and relationship root headers. Read 512-byte blocks, stopping at the root opening
tag, with a 64 KiB header budget. This is **not** a second full worksheet scan.
The shared-string root is checked even when whitespace cleanup is disabled and
its body would otherwise be copied without analysis (corrected during this review).

Explicit SpreadsheetML prefixes, Strict SpreadsheetML, UTF-16 and ambiguous or
unsupported declarations can exclude the archive. Later namespace declarations
are checked while already reading/streaming the relevant part. Ordinary x14 or
drawing extension prefixes are not equivalent to prefixed core cell elements.

An exclusion records its part and reason and produces no cleaned workbook.
“Unsupported representation” is not proof that a file was not created by Excel
or contains no bloat. Experimental namespace normalization and inspection do not
change this production eligibility policy.

Parser compatibility is narrower than all valid XML: some fast attribute and
element recognizers assume common Excel serialization. Unsupported/malformed
forms must not be described as generally normalized by the shrinker.

## S3. Profiles and switches

Default is the production preservation target. Maximum and Too Aggressive are
experimental test artifacts used to expose dependency and layout edge cases.
Their current switches remain documented for reproducibility. The intended future
product has one Default behavior; removal of experimental switches is future work.

The CLI translates user flags into core switches. In particular, the core
`CleanupSwitches::default()` is **not** the entire ordinary CLI policy: the CLI
enables deferred hidden constant-tail cleanup unless explicitly disabled. Ordinary
nonempty hidden tables are retained, including unreferenced values.

| Profile | Keep default-font representatives | Keep other font/style representatives | Keep column overrides | Remove unreferenced whitespace | Remove unreferenced hidden content | Object/reference scan |
| --- | --- | --- | --- | --- | --- | --- |
| Default | yes | yes | yes | no | only admitted constant tails | extended |
| Maximum cleaning | no | no | no | yes, subject to guards | only admitted constant tails | extended |
| Too aggressive cleaning | no | no | no | yes, subject to guards | only admitted constant tails | cell-formula-only mode |

Too aggressive is intentionally a stress profile; its weaker object dependency
coverage must not be advertised as the same preservation guarantee as Default.
Maximum can remove layout representatives and whitespace affecting overflow.
The current targeted corpus compares **Default**, not all three profiles.

Additional predefined diagnostic profiles independently remove basic/all textless
representatives, enable whitespace deletion, retain hidden content, or retain
inactive-protection cells in/all outside the data range. All profile metadata
must store resolved flags and configuration identity, not only a friendly name.

Other controls independently disable tail pruning, trailing layout-row cleanup,
extraneous-column merge or equal-fill tail merge. Empty self-closing preprocessing
is off by default. Disabling parallelism/changing chunk sizes must not authorize
different cleanup decisions.

## S4. Cell classification and decision order

The implementation recognizes content by value, formula or inline-string child
markers. A formula counts as content, including an empty shared-formula follower
tag. It must not be treated as an empty formatting cell merely because its
cached displayed value is blank. Numeric zero is content.

For each parseable cell, obtain its coordinate and explicit style, containing
row metadata, covering column definition and existing style tables. Record
content/visible bounds. Determine whether a representative, inherited style,
protection rule or deferred-reference rule prevents removal.

| Situation | Implemented decision / removal barrier |
| --- | --- |
| Visible cell with content | retain; optional whitespace deletion is a separate deferred decision |
| Formula in a hidden cell | retain its formula; not a hidden-value deletion candidate |
| Hidden cell with nonformula content | initially retain and, if eligible, defer removal pending reference scan |
| Any candidate protected by a known dependency | retain regardless of hidden/whitespace cleanup request |
| Hidden column, visible automatically sized row | retain content, including whitespace: hidden columns can influence row height |
| Hidden row, or row with both explicit height and custom-height flag | may pass the hidden-content eligibility guard; reference/protection guards still apply |
| Empty cell with effective fill/border | retain; explicit default styles can cancel inherited visible formatting |
| Empty unstyled cell clearing a row/column style | retain rather than exposing the inherited style |
| Empty font/style representative | retain under the selected representative policy |
| Unknown/ambiguous styles or parsing | follow local conservative guards; do not infer a globally proven equivalent style |
| Otherwise removable empty cell | remove or rewrite as self-closing according to the active rule |

The same font ID is not the only font comparison: byte-identical font records
have a common internal identity for decisions, without rewriting their indices.
Non-font attributes/children remain significant. A font-only XF may be reused
only when other style components cannot leak into a whole row.

### S4.1. Whitespace and blank are not interchangeable

Whitespace lookup examines shared-string entries and their text runs. Recognized
whitespace consists of ASCII space, tab, line feed and carriage return (including
empty entries). It preserves shared-string indexing; the table itself is not
compacted. This rule does not claim to recognize every Unicode whitespace or
every inline-string representation as an erasable whitespace cell.

Default leaves this optional deletion disabled. When enabled, it still requires
the reference scan, relevant protection checks and local layout barriers. A
whitespace cell may block text overflow; its removal is intentionally more
aggressive than removing a structurally absent value.

### S4.1.1. Possible future deterministic whitespace removal

Default currently keeps the optional whitespace deletion disabled. A future narrow
XML-only rule may remove a whitespace value only with a proof covering both value
semantics and presentation. Existing reference/protection guards remain necessary.
Conditional-format dependencies, formulas, names and object consumers also matter.

For overflow, resolve effective alignment and style inheritance for every possible
text source across the affected gap. Checking only the immediate left neighbor or
a cached displayed value is insufficient for merged ranges, right-to-left/right-
aligned text or formula results. Existing wrap/shrink settings are possible proof
inputs, not standalone permission to delete a blocker. Row auto-height, font
representatives, visible fill/border and inheritance-cancelling cells must also
survive. Removing a string value and removing the whole styled cell are separate
decisions. Unknown dependencies/layout retain the whitespace.

No such Default proof rule is established by the current implementation review.
Neither a general pixel tolerance nor simply enabling the experimental whitespace
switch supplies that proof. Focused cases must establish any future admitted rule.

### S4.2. Protection

Protection can be inherited from a cell, row or column XF. A protected sheet
keeps relevant protection-only cells. For an unprotected sheet, the policy is:

- `Remove`: inactive protection alone does not justify keeping the cell.
- `KeepInDataRange`: retain applicable cells inside the recorded visible range.
- `KeepAll`: retain them regardless of range, including their tail significance.

These switches concern unused protection formatting, not bypassing Excel sheet
protection or deleting a value that is otherwise required.

An empty cell with `applyFill="1"` and `fillId="0"` must remain when its
no-fill override clears a visible inherited row or column fill. White is a
visible fill too, because it suppresses gridlines. Such a cell remains a tail
anchor even without content below it. When no fill is inherited, the same
attributes alone neither justify keeping the cell nor make it a tail anchor.
Other retention rules still apply.

The regression fixture for Enron bundle `44ad86f1d9f249470fc6`, sheet `AUGUST`,
documents this distinction. Its 21,947 no-fill overrides had nothing to clear
but incorrectly anchored an otherwise content-free sheet through F2779.

## S5. Row representatives and row-style promotion

Removing empty font cells can change Excel's automatic row height and text
baseline. Default therefore keeps appropriate representatives even where no
value is visible. Representatives can be tentative in fixed-height rows and
removed if no kept row content actually needs them. Explicit merge cells and
column-format exceptions are handled separately.

A styleless empty font representative does not itself anchor a trailing region.
It remains eligible for local height/baseline preservation before later content
or another required anchor. If it masks an inherited visible row or column fill
or border, it remains a visual anchor even at the end of a sheet. White fills
receive the same protection as colored fills. Unknown explicit cell styles
remain conservative anchors.

Enron bundles `44ad86f1d9f249470fc6` and `a8bd950dff33454721b5`, sheet `NOV`,
document the content-free case. Each retained 11,632 empty cells, including 518
styleless representatives, after the no-fill override fix. Their inherited
row formats contain fonts without visible fills or borders. These representatives
must not block pruning. Removing such a content-free tail can also remove its
empty-row heights; this policy does not preserve exterior blank-grid geometry.

For dense bloat, the implementation can move an existing cell XF to the row's
`s`/`customFormat` attributes and remove redundant cells. It does not allocate a
new workbook style or change a value merely to make this transformation possible.

Promotion requires cleanup enabled, an explicitly custom-height nonhidden row,
no existing row style/custom format, no active sheet protection, sufficient row
content and valid ordered cells. Cells with unsupported attributes, unknown XFs
or non-cell content prevent this transformation.

The dominant empty-cell style must occur at least twice. The dense full-width
route requires at least 1,024 occurrences, applied font and either a nonvisible
empty-cell style or a uniform applied fill with no border/protection hazard.
The bounded dense-gap route requires at least 1,024 occurrences and at most 64
missing columns; it restores necessary original formats as explicit cells only
where the inheritance proof succeeds. Sparse promotion requires every missing
position to inherit the proposed style equivalently; merged sparse rows are not
promoted through that route. Ties choose the smaller style index.

These numeric thresholds select a representation optimization, not permission
to discard unverified values. Removing font sentinels is not equivalent to
arbitrarily deleting all empty styled cells.

After cell removal, row `spans` may be reduced to the last kept column. Empty rows
can become self-closing. Despite the cleanup-cause name, the current
`remove_irrelevant_attributes_for_empty_rows` helper returns the attributes
unchanged; the specification must not claim that this helper strips them.

## S6. Tail rows, columns and drawings

The shrinker distinguishes kept cells, rows carrying visible layout information,
and a trailing region with no required content. Content, effective fill/border,
hidden/layout attributes, applicable protection, merges and supported drawing
anchor bounds can keep portions of the tail.

Tail pruning must retain actual content; source-content counts and the tail guard
are recorded in shrink evidence. A row still needed by later content or a drawing
must not disappear solely because an earlier cell was deleted. Deferred deletions
invalidate stale tail anchors, then re-clean affected rows and reconsider the tail.

Column cleanup **can change column XML**. It collapses extraneous tail definitions
past the recorded maximum used column; it is not a general operation to narrow
columns containing data or delete/reindex data columns. Any hidden column
definition disables the relevant collapse. Definitions crossing the data boundary
are not split by this cleanup.

The style-aware route can collapse a visible equal-fill tail into the first
compatible definition or a default tail into the last compatible definition.
Visible borders and incompatible styles restrict eligibility. Changes to inherited
column fonts/styles may still affect row layout, hence the separate effective
height checks during validation. “The columns were empty” alone is not a visual
preservation proof.

Drawing-anchor bounds are additional layout anchors. The shrinker's drawing
lookup is not a full rendering engine: missing/unreadable or unsupported drawing
information is a remaining review concern, not a proof that no object exists.

## S7. Deferred reference-sensitive removal

Hidden-content admission is deliberately narrower than “hidden and unreferenced”.
During the initial row traversal, original cells are observed before style cleanup.
Only a dense hidden suffix of at least 4096 equal signed-integer constants ending
at XFD qualifies. Text, formulas, unsupported numeric forms, gaps, changed values,
and missing cell-level eligibility retain the suffix. The threshold is a conservative
heuristic, not a probability or a proof that a constant matrix was unintended.
Only horizontal full-width tails qualify; short tails and vertical runs remain.

Eligible Hidden tails become compact row spans. If any known reference intersects
a span, the whole span survives. Nonqualifying Hidden candidates are dropped before
the workbook reference scan. Whitespace retains its separate policy. The span list
is merged in row-chunk order and reference-filtered before the existing XML rewrite;
there is no extra full-sheet traversal or expansion into a coordinate hash table.

First retain eligible hidden/whitespace content as candidates. Scan the relevant
workbook parts, then remove only candidates outside the protected sets. This
two-stage process is essential: a reference on a later or another worksheet must
protect a candidate already encountered during initial cleanup.

Protected information includes known cell references and rectangles, local and
sheet-qualified formulas, names, supported non-selection `ref`/`sqref` consumers,
and (in extended mode) chart/control/object formulas and pivot source/result
dependencies. Layout-only dimensions, selections, print/filter extents and
drawing anchors are not automatically value consumers.

Shared formulas require translation from the anchor across the declared follower
range. Relative axes move; `$`-fixed axes do not. Keep enclosing referenced
rectangles without enumerating every follower. Incomplete/conflicting shared
groups, out-of-grid translation, dynamic functions or unresolved sources make
reference-sensitive cleanup unsafe and retain candidates. Known blockers include
`INDIRECT`, `OFFSET`, `ADDRESS` and `CELL`; structured or named object sources can
also prevent a safety proof.

Sheet scope matters. A local `A1` formula on another known sheet must not be
silently assigned to the candidate sheet; unresolved scopes use conservative
protection. Unsupported reference syntax is not evidence of “unreferenced.”
See [the shared-formula methodology](../methodology/reference-protection.md).

Removing a deferred content value can retain the cell's formatting representation
where needed. The touched rows then undergo normal cleanup again. A failed
reference scan disables deferred removal; it does not erase candidates anyway.

## S8. Workbook names and calculation chain

The built-in alias cleanup recognizes `Print_Area`, `Print_Titles`,
`_FilterDatabase` and `Database` when their `_xlnm.` counterpart exists in the
same raw `localSheetId` scope. It removes the unprefixed alias and keeps the
prefixed counterpart. Missing local scope denotes workbook scope, not sheet 0.
Attribute order and quote style must not change this decision (corrected during
this review); malformed/duplicate identifying attributes prevent the match.

This is an alias-name-and-scope policy. The implementation does **not** establish
that their formula bodies are equal. Treat that stronger safety question as an
open requirement, not an implemented content-equivalence check.

The CLI can clean calculation-chain entries for removed cells, resolving sheet
identifiers through workbook relationships. Unknown mappings are not permission
to drop entries. Formula evaluation/calculation caches are otherwise Excel's
responsibility, not produced by this shrinker.

## S9. Errors, output identity and observability

An unsupported representation aborts output. Other local read/cleanup/compression
failures can preserve an original entry and record fallback/error information.
Such a file must not be described as every requested transformation succeeding.
Writer failure must not publish a truncated XLSX as a valid cleaned result.
Batch callers must inspect per-file outcomes/evidence; a returned batch invocation
alone is not a guarantee that every input produced a cleaned output.

Byte searches and row-aligned state machines are used for large worksheet paths;
regular expressions remain in other recognizers, including reference and workbook
name matching. This specification does not claim the entire codebase is regex-free.

Reusable-entry detection can avoid recompression; shrink evidence stores CRC and
uncompressed-length information, package fingerprints, worksheet changes and
source-content/tail observations. These are byte/package observations, not
cryptographic proof of semantic equivalence of every XML construct.

Acceptance cases include: unchanged values/formulas; visible zero; protected
hidden formulas; translated shared references; a hidden-column font affecting an
automatic row; default fill cancelling inherited fill; merge representatives;
dense-row gap restoration; a drawing below the last data row; and defined names
on different sheets with reordered attributes.

## S10. Future Excel-assisted post-cleanup

This is a proposed separate Excel add-in stage, outside the Rust shrinker. The
XML-only Default pass first performs the large deterministic reduction, improving
the cost of opening the workbook for later Excel-side work.

A possible font-representative transaction would record effective row heights and
baseline images in Excel, assign explicit custom heights, remove only redundant
font representatives, then capture the affected presentation again. Excel supplies
the effective heights; a guessed rendering model or saved `ht` alone is insufficient.
The current appearance is the preservation target. Freezing automatic height also
changes behavior under later user edits and needs an explicit opt-in contract.

A possible whitespace transaction would retain baseline evidence, remove a proved
unneeded whitespace value, and compare again. If it was an overflow blocker,
trial a supported Excel formatting replacement and recheck the affected area,
row geometry and dependent objects. The desired outcome is hidden overflow;
CSS `overflow: hidden` is not an established Excel cell-style operation. Office.js
exposes alignment, wrap, shrink-to-fit and row height, whose visual effects require
validation. Reject/revert a trial with a meaningful difference or incomplete check.
Known rendering variation follows the visual acceptance policy.

The proposed stage may then request a full rebuild and save through Excel.
Excel may also reduce unused shared strings/styles during saving; the amount of
compaction must be observed rather than guaranteed. Independent rewriting,
deduplication or renumbering of those tables is outside the core product scope.
Native save/rebuild behavior is reused instead.

Retain distinct identities and evidence for raw input, Rust output, ordinary Excel
save, add-in edits and rebuild/save. This separates reductions and changes caused
by each stage. A future add-in must preserve a rollback source and publish only
a verified successful trial. Captures establish the observed presentation, while
dependency checks still protect semantic uses of apparently blank values.

Office.js API references for the proposed stage are [RangeFormat](https://learn.microsoft.com/en-us/javascript/api/excel/excel.rangeformat?view=office-js)
and [Range.getImage](https://learn.microsoft.com/en-us/javascript/api/excel/excel.range?view=office-js#excel-excel-range-getimage-member(1)).
These APIs establish available operations, not the equivalence of a trial edit.

## Source map

| Rules | Main implementation / focused tests |
| --- | --- |
| S1-S3, S7-S9 | `excel-shrink-cli/src/pipeline.rs`, `visual_profile.rs`, `pipeline_namespace_tests.rs`, `pipeline_regression_tests.rs` |
| S2 | `excel-shrink-core/src/xml/namespaces.rs`, `xml/namespaces/header.rs` |
| S4 | `sheet/cells.rs`, `sheet/cells_regression_tests.rs`, `shared_strings.rs`, `shared_strings_tests.rs`, `styles.rs` |
| S5 | `sheet/rows.rs`, `sheet/rows_regression_tests.rs`, `sheet/row_view.rs` |
| S6 | `sheet/ranges.rs`, `sheet/cols.rs`, `sheet/drawings.rs`, related range/tail tests |
| S7 | `reference_scanner.rs`, `reference_scanner/shared_formulas.rs`, `reference_scanner/object_references.rs`, scope/regression tests |
| S8 | `workbook.rs`, `workbook::scope_tests`, core `tests.rs` |

Core paths in the final five rows are relative to `excel-shrink-core/src/`.
