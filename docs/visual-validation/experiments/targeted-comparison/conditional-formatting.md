# Conditional-formatting capture selection

The targeted comparison records a decision for every conditional-formatting block
in every available state pair. This selection is part of the existing retained-state
pipeline, including save and rebuild controls. It does not run a second full-sheet sweep.

## Constant `cellIs` rules

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
`styles`, `dummy-missing`, `dummy-inert`, `dummy-present` and `dummy-added`.
The verifier checks selection decisions, actual pixel differences, red/blue
differential fills, and owned-process closure. Add `-CheckPerformance` to exercise
the six-state pipeline. `dummy-inert` removes the missing-target rule in the partner:
Excel must render the same pixels even though the changed catalogue requires capture.

A separate ignored test accepts `CF_ORIGINAL` and `CF_SHRUNK` local paths and checks
retained missing-Dummy evidence after actual shrinking. It was exercised on inventory
source `08288.xlsx` (the Data.gov primary-school model): all four CF blocks retained
the missing-target proof. This was an XML selection test, not a full native visual
comparison of that model. Corpus files and generated evidence stay uncommitted.
