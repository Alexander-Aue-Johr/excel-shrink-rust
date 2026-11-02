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
Whole-column/whole-row references, sheet qualifiers and range unions/intersections
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
sheet-qualified/structured references and unsupported tokens retain the full-area
fallback. A formula nesting guard rejects excessive recursion into the same
fallback; it does not limit sheet size or candidate count.

The report adds `referenced_input_changes` and `referenced_formula_observations`.
The latter counts appearances in both states, not distinct cells. Unknown formula
syntax, sheet-qualified references and names remain full-area fallbacks.

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
