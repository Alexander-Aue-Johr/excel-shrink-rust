# Dependency-based chart capture

## Decision

A chart data cache alone is insufficient. Identical cached data can coexist with changed worksheet inputs. The deterministic mode requires unchanged chart XML/cache, a closed supported package graph, source-cell fingerprints and styles, source visibility/layout and supported CF dependencies, plus complete identical live chart placement. Unknown cases retain full-chart exports. Conservative mode exports every chart.

The parser reads XML bytes with the existing borrowed token reader and hierarchy depth. Only attribute values and formula text are decoded. It does not build an XML DOM or evaluate Excel formulas. A separate inventory command retains all chart XML parts by SHA-256 so unknown formulas and extensions can be reviewed without rescanning worksheets.

## Lysimeter

The Cotton 2020 Lysimeter test contains 69 charts. The production gate excludes 34 and retains 35 captures per state. Eighteen fallback objects have ambiguous duplicate names. Seventeen reference userShapes drawings, whose additional data dependencies are not yet proved. Identical caches are deliberately insufficient to exclude those objects.

This gate test uses fresh disposable COM-rebuilt workbook copies and live COM chart geometry. A complete replacement Lysimeter image run was not performed here. Source hashes and the preceding cell-selection experiment are documented in the adjacent lysimeter-rebuilt-selection experiment. The retained local gate evidence is under V:/excel-shrink-paper/external/excel-shrink-rust/.tmp/validation-temp/lysimeter-value-audit.

## Native Office.js controls

A workbook created by Excel contains Data!A1:B3 and one column chart. A second ZIP differs only in document description metadata. The private-desktop production launcher runs three cases with SQLite enabled.

| Case | Chart images | Dependency exclusions | Chart pixel differences |
| --- | ---: | ---: | ---: |
| Unchanged, deterministic | 0 | 1 | 0 |
| Unchanged, conservative | 2 | 0 | 0 |
| Changed source value, deterministic | 2 | 0 | 1 |

The nine produced SQLite databases passed integrity_check. An earlier synthetic metadata edit lost an XML namespace declaration and caused Excel to request repair. The normal blocker rejected it. The fixture was corrected to preserve original namespace declarations before the successful controls above.

## Entry points

Use --chart-capture-mode deterministic or --chart-capture-mode conservative with regression targeted-compare or regression study. The PowerShell private-desktop wrapper accepts -ChartCaptureMode. Corpus launcher settings use chart_capture_mode. Reports retain the mode and per-pair chart_dependencies decisions. Actual exclusions have dependencies_unchanged=true and pixels_compared=false. Historical full-chart results remain reusable; explicit conservative requests do not reuse records created in deterministic mode.

Run regression chart-inventory --input INPUT.xlsx --output NEW_DIRECTORY for one file, or supply a corpus ZIP. An optional --limit bounds a trial by workbook count. The output retains raw XML, provenance rows, cache-node counts, formulas, simple A1 reference counts and unsupported syntax reasons. Inventory eligibility is not a workbook invariance proof.

## Verification

Focused Rust tests cover changed source values with unchanged caches, styles, row visibility, themes, cached chart values, missing partners and geometry, unsupported references, namespace and extension payloads, duplicate names, and references outside worksheet capture bounds. Report-contract tests reject exclusions without matching receipts. JavaScript tests verify geometry inventory without image exports. The normal targeted and archive-result test groups pass, with live/manual experiments left explicitly ignored unless their input paths are supplied.
