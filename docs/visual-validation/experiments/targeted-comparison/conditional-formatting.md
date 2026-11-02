# Conditional-formatting capture selection

The targeted comparison records a decision for every conditional-formatting block
in every available state pair. This selection is part of the existing retained-state
pipeline, including save and rebuild controls. It does not run a second full-sheet sweep.

## Constant `cellIs` rules

Namespace eligibility is retained per CF subtree, not per sheet. An unchanged
extension block keeps its own capture fallback without disabling a standard sibling.
The worksheet scan tracks default-namespace scopes, direct-child placement and
foreign/prefixed descendants. Eligibility changes invalidate the catalogue match
even when the raw CF bytes are identical. Changed rule catalogues still retain the
existing conservative old/new-area fallback.

Unchanged supported literal rules depend on local cell inputs. The selector compares
the complete raw rule catalogue, styles, shared strings, themes, workbook properties
and worksheet layout context before excluding their unchanged inputs. It reuses the
streamed cell XML changes; it does not build another in-memory cell index.

The input comparison is deliberately stricter than semantic value equality. A cell
XML change remains a candidate, including deletions, type differences and style-only
changes. Blank, zero and empty strings are not collapsed. Formula-bearing inputs
remain captures even when their cached values and XML match. Changed row formatting
selects the affected row within the rule area. Other layout/context changes fall back
to capturing the application area.

Only recognized numeric or quoted-string literals and supported `cellIs` operators
qualify. References, expressions, unknown content, uncertain namespaces and truncated
formula summaries cannot establish this proof.

## Local reference dependencies

`LEN(TRIM(A1))=0` supports relative, absolute and mixed A1 references. The selector
inverts each changed input into dependent target rectangles; it never evaluates
LEN, TRIM or the comparison. Deleted inputs and formula-bearing inputs with equal
caches are included. Existing local-input, row-layout and visual-context guards
also remain in force.

The first sqref area supplies a shared relative anchor. For disjoint areas the
selector additionally includes per-area anchors conservatively, potentially
capturing extra cells. Absolute axes can select an entire target row, column or
area. Selection uses rectangle arithmetic, without enumerating every target cell.
As with other CF captures, targets are clipped to the recorded comparison extent.

The same reference mapping supports simple scalar formulas in expression, text,
blank/error and `cellIs` rules. A byte parser consumes the complete formula and
collects every explicit local A1 reference, including references in all IF branches.
Quoted strings (with doubled quote escapes) never supply cell dependencies.
Arithmetic, comparisons, concatenation, percentages and numeric exponents are
recognized without evaluating their results or operator precedence.

Rectangular local ranges such as `A1:B10`, `$A$1:$B$10` and `$A$1:A1` are
tracked as dependency rectangles. Mixed endpoint axes and reversed endpoint order
are handled by inverting both possible endpoint orders. Changed inputs select all
targets whose translated range contains them, without enumerating range cells.
Whole-column/whole-row references and range unions/intersections
remain unsupported. Native controls `ref-range`, `ref-range-fixed` and
`ref-range-growing` check sliding SUM, fixed COUNTIF and growing SUM dependencies.

Supported functions are LEN, TRIM, ISERROR, ISERR, ISBLANK, ISTEXT, ISNUMBER,
SEARCH, FIND, LEFT, RIGHT, MID, AND, OR, NOT, IF, IFERROR, MOD, ABS, ROUND, INT,
LOWER, UPPER, EXACT, CODE, SUM, MIN, MAX, AVERAGE, COUNT, COUNTA, COUNTBLANK,
COUNTIF and COUNTIFS. Their dependencies are their explicit arguments.
SUMIF/SUMIFS remain unsupported: in particular, SUMIF can extend a small sum-range
argument to the size of its criteria range, beyond the explicitly written cells.
See [Microsoft's SUMIF reference](https://support.microsoft.com/en-us/excel/functions/sumif-function).
INDIRECT, OFFSET, ambient/volatile functions, unknown functions, names,
structured/external references and unsupported tokens retain the full-area
fallback. A formula nesting guard rejects excessive recursion into the same
fallback; it does not limit sheet size or candidate count.

The report adds `referenced_input_changes` and `referenced_formula_observations`.
The latter counts appearances in both states, not distinct cells. Unknown formula
syntax and names remain full-area fallbacks.

Simple same-workbook qualifiers are supported for cells and rectangular ranges:
`Other!A1`, `'Start here when entering data'!$D$19` and
`'O''Brien data'!$A$1:$A$2`. Literal ASCII sheet names resolve uniquely and
case-insensitively in both snapshots. Each source worksheet is checked once for
changed inputs and uncertain formula cells; resulting captures belong to the target
worksheet. No referenced formula is evaluated.

Missing, ambiguous, renamed, hidden/unavailable and unsupported-namespace source
worksheets retain the application-area fallback. Encoded `_x` names, non-ASCII
qualifiers, external workbooks, 3-D references and repeated endpoint qualifiers
remain unsupported. The source sheet must already be present in the visual snapshot;
this change does not load extra worksheets or change hidden-sheet handling.
Native controls `ref-cross-relative`, `ref-cross-quoted` and `ref-cross-range`
verify relative coordinates, escaped apostrophes and foreign-sheet range inputs.

Excel promotes these cross-sheet rules to x14 expressions when saving. A narrow
adapter supports that representation inside a known x14 conditionalFormattings
container, with validated x14/xm namespace bindings. It accepts expression formulas
and whitelisted static inline DXF formatting only; original subtree fingerprints
still cover the formula, IDs and complete inline format bytes. Unknown extension
nodes and other x14 rule types keep their fallback. Data bars have the separate
whole-area dependency support described below. The adapter does not
make general extension rules eligible.

Native controls `ref-relative`, `ref-column`, `ref-row`, `ref-fixed` and
`ref-disjoint` verify dependent target pixels in Excel after changing a source
outside the target area.
Additional controls `ref-search`, `ref-multiple` and `ref-cell-is` check nested
SEARCH/ISERROR, multiple dependencies and a foreign-cell `cellIs` operand.

## Whole-area dependencies: duplicates and color scales

Mixed blocks combine the already supported local, whole-area, explicit-reference
and missing-Dummy procedures. Every rule must be understood. Their dependency
footprints are unioned, without evaluating rule outcomes, priorities or stopIfTrue.
Those rule attributes must still match in the complete raw catalogue.

A changed/uncertain whole-area input selects the entire application area. If only
an external-to-area local reference changes, its dependent targets are selected;
an unchanged whole-area rule does not widen that selection. Missing-Dummy members
contribute no footprint only when their absence proof succeeds in both states.
One unknown member retains the full block fallback.

Native controls `ref-mixed` and `ref-mixed-group` distinguish reference-driven
partial selection from whole-area input changes, including Check Performance.
Report reasons starting with `combined-` identify these decisions.

Unchanged `duplicateValues` rules and standard color scales can exclude their
application area only when all its cell XML inputs and the visual context match.
Color scales support fixed numeric, minimum/maximum, percent and percentile
thresholds. Formula thresholds and unknown threshold types remain full-area
fallbacks; no formula evaluator or general reference resolver is introduced.

Any changed or deleted input, or any uncertain formula-bearing input, captures
the entire dependent application area within the comparison extent. The selector
does not calculate whether the changed input would actually alter the formatting.
Changes outside the application area do not trigger this dependency. Mixed blocks
containing supported literal rules use the same conservative whole-area decision.
Report reasons distinguish unchanged whole-area inputs from changed/uncertain
dependencies. Existing row-layout and context guards remain in force.

## Fixed missing-Dummy template

The only supported INDIRECT exception is a `cellIs` / `notEqual` rule containing:

```text
INDIRECT("Dummy_for_Comparison<digits>!"&ADDRESS(ROW(),COLUMN()))
```

Case and whitespace outside quoted strings are normalized. XML entities are decoded
once. The target must be absent from both complete workbook sheet-name catalogues,
including hidden sheets. Uncertain name encodings, CDATA, changed rule catalogues,
modified expressions, existing targets and newly added targets remain captures.
This is not general INDIRECT evaluation or reference analysis.

## Evidence and limits

`report.json` adds `conditional_formatting` entries with left/right state IDs and
per-block decisions. Each decision records its outcome (`excluded`, `partial` or
`capture`), reason, original `sqref`, catalogue side and comparison extent. Counts
for changed inputs, formula observations and changed layout rows explain partial
selection. Formula observations count appearances across both states, not unique cells.
The Observatory summary shows decision counts across state pairs.

Unresolved application areas are intersected with the union of the two existing
visual-plan extents and tiled into rectangles of at most 128 rows by 16 columns.
A missing/unresolved application range uses that entire comparison extent. Rectangular
candidates use the existing `cell` report field, for example `A1:P128`; point candidates
retain their existing padded-cell captures. The rectangle count has no global cap.
Existing Excel image API constraints still apply.

An excluded block proves only that supported CF contribution unchanged. It is not
a claim that an entire sheet or workbook was visually compared or proven identical.
Other candidate, chart and row-height checks remain independent. Regions outside the
existing comparison extent are not newly covered by this change.

## Verification

Run focused Rust checks with `cargo test -p excel-shrink-cli targeted_comparison`
and `cargo test -p excel-shrink-cli cf_inventory`. The existing ignored release
streaming checks cover a million-cell sheet and a single XML row exceeding the old
fragment size; neither a cell-count nor an XML-size cap is introduced.

`run-smoke.ps1 -ConditionalFormatting <mode> -Output <fresh-directory>` runs native
Excel controls on a private desktop. Modes are `constant`, `constant-string`,
`styles`, `dummy-missing`, `dummy-inert`, `dummy-present`, `dummy-added`, `duplicate`
and `color-scale`.
The verifier checks selection decisions, actual pixel differences, red/blue
differential fills, and owned-process closure. Add `-CheckPerformance` to exercise
the six-state pipeline. `dummy-inert` removes the missing-target rule in the partner:
Excel must render the same pixels even though the changed catalogue requires capture.

A separate ignored test accepts `CF_ORIGINAL` and `CF_SHRUNK` local paths and checks
retained missing-Dummy evidence after actual shrinking. It was exercised on inventory
source `08288.xlsx` (the Data.gov primary-school model): all four CF blocks retained
the missing-target proof. This was an XML selection test, not a full native visual
comparison of that model. Corpus files and generated evidence stay uncommitted.

## Data bars

Standard `dataBar` rules now use the same whole-application-area input proof as
color scales and duplicate-value rules. Rule fingerprints, input cell XML,
formatting context and relevant layout must be unchanged to exclude the CF area.
Any changed/deleted input or uncertain formula input captures the dependent area;
the selector never predicts whether the resulting bar would look different.
Mixed blocks union their known dependencies through the existing selection path.

Standard thresholds support min/max and finite numeric, percentage or percentile
constants. Formula thresholds, unknown settings or children remain fallbacks.
Known x14 data bars with autoMin/autoMax or min/max support fixed appearance
settings, negative colours and axis settings. Other x14 thresholds remain
conservative, including numeric values encoded as formula children.

Legacy/x14 pairs must have one unambiguous matching ID and the same application
range within the worksheet. Missing companions, duplicate IDs, mismatched ranges,
foreign namespace bindings and unsupported companions retain capture fallbacks.
No input workbook is rewritten: normalization adapts only the selection model;
raw fingerprints still include every setting, colour, ID and range.

Focused tests cover unchanged/changed/deleted/outside inputs, formula caches,
numeric thresholds, mixed rules, changed styles/rules, missing/duplicate/mismatched
links, unknown settings, namespace failures and formula thresholds.
The private-desktop native controls `data-bar` and `data-bar-unchanged` passed.
The positive control changes a maximum and verifies different bar pixels in other,
unchanged cells; the negative control excludes the CF dependency. Other pipeline
candidate reasons remain independent and can still produce captures.

The repeated Data.gov eligibility audit (2026-11-02, 107.42 s, zero errors) moved
all 256 pure-data-bar blocks and four mixed bar/scale blocks onto supported
whole-area proofs: structural fallbacks decreased from 1,092 to **832** out of
20,935 blocks. This is eligibility, not a claim of unchanged inputs or a runtime
speedup. See `../cf-inventory/data-bars-20261102.json` for measured counts and
source-based fallback footprints.

## Bare `isblank` in the PCC workbooks

Both PCC 2017/2018 workbooks contain five expression rules with the bare token
`isblank` and two with the quoted string `"isblank"`. The string literals already
have no external reference dependency. The bare token is a defined-name lookup,
not an implicit call to `ISBLANK(cell)`. Neither source defines that name.

The new input-independent proof applies only when the bare token's workbook- and
sheet-scoped name is provably absent in both states. Hidden/local names are
included; uncertain entity or `_xHHHH_` encodings keep the fallback. Excel's
ordinary `_xlnm` built-in names do not make the catalogue uncertain. Existing or
newly added names retain captures; general name resolution is not implemented.
Rule fingerprints, formatting context and relevant layout must still match.
Changed cell values are handled by the independent ordinary candidate selector.
Mixed blocks retain every other supported dependency.

Private-desktop controls `name-inert`, `name-missing` and `name-defined` passed:
removing the undefined-name rule produces identical pixels; leaving it unchanged
permits CF exclusion; defining the name as TRUE versus FALSE produces the expected
visible red-fill difference and keeps captures. All owned Excel processes closed.
Unit controls also distinguish bare `isblank` from the reference-bearing
`ISBLANK(A1)`, and cover local/hidden names, added names and uncertain catalogues.

Workbooks containing Excel tables keep the bare-name fallback, since table identifiers
can also resolve in formulas. The guard checks normal table parts and declared
custom table locations; missing/uncertain package metadata also keeps the fallback.
Both PCC sources were checked and contain no table parts. This is intentionally
more conservative than implementing a table-name resolver.

## Constant reference-error expressions

The initial support covered complete expression forms `#REF!` and `#REF!=1` as
input-independent error expressions. Further forms are documented below. Optional leading `=` and surrounding
whitespace are accepted; XML entities are decoded once. Additional cell references,
operators or functions remain unsupported. There is no formula evaluator and no
attempt to repair broken references. Selected cellIs operands are covered by the
separate extension below; chart expressions and formula-valued color-scale/data-bar
thresholds remain outside this proof.

Unchanged catalogue fingerprints, namespace support, application ranges, formatting
context and relevant layout remain required. A changed rule or context keeps its
capture fallback. Ordinary changed cells still remain separate capture candidates;
mixed blocks retain their other dependencies. Error proofs do not depend on a
complete name catalogue, since the two literal errors contain no name reference.

The native controls `error-ref-inert`, `error-ref-compare-inert` and
`error-ref-visible` passed on a private desktop. Both error forms match a removed
rule; replacing the error by a valid true expression produces the expected red
fill. Additional stacked controls prove that an error with stopIfTrue does not
suppress a subsequent valid blue-fill rule. Owned Excel processes were confirmed
closed. Unit tests reject expressions such as `#REF!+A1`, `Other!#REF!`,
`ISERROR(#REF!)` and `#REF!=1+NOW()` and retain style/catalogue change guards.

The repeated Data.gov static eligibility audit completed with zero errors:
**832 -> 203 structural fallbacks**, with 10 newly supported missing-isblank blocks
and 619 constant-error blocks. Source-clipped candidate rectangles decreased from
**1,698 to 558**. These are static estimates, not measured runtime speedups or a
claim that every supported region was visually compared. The later table guard
was checked against the two actual PCC sources; they are the only bare-isblank
sources in the inventory and remain unambiguous. Full results are in
`../cf-inventory/name-errors-20261102.json`. The production corpus remains stopped.

## cellIs comparisons against a literal #REF! operand

The six single-operand comparison operators (equal, notEqual, lessThan,
lessThanOrEqual, greaterThan and greaterThanOrEqual) now recognize an exact
`#REF!` operand as input-independent. The comparison produces an error rather
than enabling its format. The complete rule catalogue, formatting context and
layout guards remain required. Ordinary changed-cell captures remain independent.
Between/notBetween, arbitrary operand expressions, unknown settings and qualified
external references retain their existing fallbacks.

Unit tests cover all six operators, changed inputs, changed styles/rules and
rejected external/compound operands. The opt-in retained-source check exercises
all 16 actual Data.gov cases and verifies that the one external workbook reference
is still unsupported. Native controls `cell-is-ref-inert` and `cell-is-ref-visible`
passed on a private desktop: all six operators were exercised with numeric and
text cells, stopIfTrue and a subsequent blue-fill rule. Removing the error rules
leaves the same pixels; replacing the error operands with valid constants produces
red fills. All owned Excel processes closed. This is not a new full corpus run.

## Top-/bottom-ranking rules

Standard top10 rules with explicit positive integer rank, optional boolean bottom
and percent settings, and no formula/extension payload now use the existing
whole-area input proof. No ranking is evaluated. Equal rule/context and complete
application-area inputs exclude the CF region; any changed/deleted or uncertain
formula input selects the complete dependent area. Unknown settings remain fallback.
Count and percentage variants, mixed blocks, outside inputs and deleted inputs
are covered by focused tests. Native controls `top-bottom` and
`top-bottom-unchanged` passed, including visible indirect changes in unchanged
ranked cells. Other candidate reasons remain independent. Owned Excel copies closed.

## Physical row-parity expressions

The exact expression forms `MOD(ROW(),2)=0` and `MOD(ROW(),2)=1` are input-independent
at fixed worksheet coordinates. Token-level matching permits ordinary whitespace
and case variations without joining identifiers or accepting extra dependencies.
ROW(cell), variable divisors and appended functions retain fallback. Catalogue,
application-range and formatting/layout guards remain in force; ordinary changed
cells and geometry differences still have their own capture selection.
Unit tests include mixed dependencies and malformed/suffixed formulas. Native
controls `row-parity` and `row-parity-shift` passed: value edits retain band pixels
outside the edited row while still capturing the value change; changing parity
captures and reveals the changed bands. Owned Excel processes closed.

## Additional fixed error expressions and text operands

`ISTEXT(#REF!)` is recognized as a constant false test. The literal `#DIV/0!` and
`#REF!` equality comparisons with complete Excel string literals are recognized
as constant errors. A doubled quote is the only quote escape accepted inside a
text literal. Concatenation, appended functions, additional references, incomplete
quotes and other operators stay conservative. This is a small recognition set,
not formula evaluation. Recognition uses ASCII token whitespace; non-ASCII name
characters cannot be silently removed to construct a supported function name.
Catalogue, application range, namespace, formatting/context and layout guards
remain unchanged. Mixed blocks keep their other dependencies and changed cells
remain separate capture candidates.

Native `static-error-inert` and `static-error-visible` controls passed with all
four concrete forms (ISTEXT, division error, Valid and Cancelled text operands).
Error/false rules match their removal, do not suppress a subsequent blue rule
under stopIfTrue, and a true-expression replacement renders the expected red fill.
All owned Excel processes closed. Unit controls include escaped/empty strings and
hidden concatenation/reference suffixes; 29 focused CF tests passed.

The refreshed Data.gov eligibility audit completed in 107.96 seconds with zero
errors: **187 -> 91 structural CF fallbacks**, and **542 -> 276 source-clipped
candidate rectangles**. These are static selection estimates, not runtime gains
or visually validated workbooks. Two ranking rules in Pensacola remain because
they are pivot-bound; the existing pivot and external-reference guards remain.
See `../cf-inventory/ranking-parity-errors-20261102.json` for the complete summary.
The production corpus remains stopped.
