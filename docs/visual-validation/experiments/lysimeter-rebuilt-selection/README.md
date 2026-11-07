# Direct rebuilt-state selection on Cotton Lysimeter

## Question

How much capture work remains when selecting from rebuilt Original versus rebuilt Default, without adding candidates from each opening/saving control?

## Observation

The production report selected 533 cell-image regions per state, derived from 3,260 internal grid tiles. Direct selection produced 135,632 cell candidates, 1,061 grid tiles and 11 coalesced rectangles. Measuring and partitioning those rectangles produced 143 image regions per state. The existing 69 whole-chart observations remain additional work. This is a selection experiment, not a measured end-to-end speedup or a new visual-validation result.

All decoded cell-value streams and formula serializations matched across the two rebuilt copies. Shared strings were resolved and decimal numeric spellings normalized. Effective Excel row-height runs matched on 11 sheets, covering 1,113,516 row positions per state. All seven conditional-formatting blocks were excluded by the existing unchanged-input proof.

The rebuilt Original contains 6,651,371 cell elements, including 2,322,835 empty cells. Rebuilt Default contains 4,471,927 cell elements, including 143,391 empty cells. The highest actual row is 114,054. Large cell counts do not imply rows outside Excel's grid.

All 69 chart XML parts and their caches are byte-identical between the test states. Drawing parts, chart relationships and theme parts also match. This inventory alone does not establish a complete chart dependency proof.

## Method and limits

The completed production attempt had already removed its workbook copies. Fresh disposable copies were extracted from its retained prepared bundle, opened independently through COM, fully rebuilt with CalculateFullRebuild and saved normally. Production inputs and results were unchanged.

The ignored Rust diagnostic uses the production byte-stream snapshots, resolved style identities, cell-change and conditional-format selection, content clipping and region coalescing. Style properties replace numeric style indices. Conservative XML fingerprints can still select semantically equivalent structures. This is not a complete effective-style inheritance evaluator.

Effective heights and region geometry were measured through COM in the disposable rebuilt workbooks. The local partition harness follows the production 2 pixels/point estimate, 32 MP budget, 30,000-pixel edge limit and fan-out rules. Office.js geometry and actual getImage exports were not rerun for this alternate plan. The 143 figure is the geometry-based test plan, not an observed production capture count. Column-definition changes in this pair concern consolidation or widths outside the content envelope; effective heights match. Other drawings and arbitrary layout dependencies remain subject to the existing pipeline's documented limits.

## Repeating the XML diagnostic

Set XML_DIAG_LEFT and XML_DIAG_RIGHT to retained rebuilt XLSX paths. Set XML_DIAG_OUTPUT to a writable JSON path and TEMP/TMP to the intended scratch volume. Then run cargo test -p excel-shrink-cli inspect_retained_state_selection -- --ignored --nocapture.

The output distinguishes internal grid tiles, coalesced regions, CF decisions and XML change locations. Final image counts additionally require live geometry measurements. The local full test files and harnesses are retained under V:/excel-shrink-paper/external/excel-shrink-rust/.tmp/validation-temp/lysimeter-value-audit. Workbook SHA-256 identities and compact measurements are in results.json.
