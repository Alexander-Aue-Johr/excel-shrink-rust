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


## Compatibility and migration

A byte inventory inspected all 8,623 prepared originals and found 66 possible
row-fill masking cases. Replaying Default and comparing SHA-256 for every
decompressed package member found exactly one changed workbook, this Carbon/
Timber case. The other 65 outputs were unchanged; no replay failed.

The previous output was 551,856 bytes and the corrected output is 557,937 bytes,
compared with 570,324 bytes for the source. Across the three affected worksheets,
690 compact row headers survive again. Empty-cell counts match the earlier
Default output; no original empty-cell blocks were restored.

The corpus runner was drained, the single prior bundle and record backed up
with verified hashes, and that bundle invalidated. Existing comparison reports
and images remain intact. The new build resumed with four groups, the existing
1,024 MiB reserve and all other proven-compatible results. Local inventory,
replays and migration plan use the `row-fill-cache-` prefix under the temporary
root. The verified backup is
`backups/before-row-fill-masks-20261106-070207`.
