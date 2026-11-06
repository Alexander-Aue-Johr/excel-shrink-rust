# Empty row headers masking column fills

## Observed case

`carbon_timber_analyses final_6a8f6638405f.xlsx`, Data.gov group
`db79a2d0e56b9c4b89a9`, had gray regions after shrinking where the original was
white. Three timber-value sheets were affected. Their no-fill row style uses
`fillId="0"` and `applyFill="1"` and overrides gray column formatting.

A counterfactual restored only 303 empty row headers, rows 390 to 692 in
`xl/worksheets/sheet7.xml`, to the old Default package. The difference on
` Timber Value 22 (Tons)` disappeared. The other two affected sheets retained
their differences. This isolated the row-header removal without restoring empty
cell masses or modifying styles, values, column widths or row heights.

The production correction extends the existing column-border masking rule to
applied no-fill row formats. It checks effective column boundaries, including
exact overrides, and preserves row headers as visual anchors. The same anchors
participate in range-tail pruning. Explicit empty cells remain independently
removable.

The full paired-rebuild comparison of the corrected output completed with all
17 selected regions matching and no chart differences. Native probes ran on
owned private desktops and retained their inputs and reports under
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp\carbon-row-fill-mask-proof`.
The source corpus package is unchanged. The result retains selected-region
coverage limits.
