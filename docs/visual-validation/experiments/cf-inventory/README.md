# Offline conditional-formatting inventory

The current selector's remaining fallbacks and source-clipped cost ranking are
recorded in [current-fallbacks-20261102.md](current-fallbacks-20261102.md).
The subsequent data-bar implementation reduces structural fallbacks from 1,092
to 832; its updated measurements are in [data-bars-20261102.json](data-bars-20261102.json).
The following narrow missing-name and constant-error proofs reduce that to 203;
see [name-errors-20261102.json](name-errors-20261102.json).

The first complete Data.gov run and its concrete fallback findings are recorded
in [datagov-20261101.md](datagov-20261101.md).

Run without Excel, shrinking, recalculation or changing corpus results:

```powershell
target/release/excel-shrink-cli.exe regression cf-inventory --input <corpus.zip> --output <new-directory>
python docs/visual-validation/experiments/cf-inventory/report.py <inventory-directory>
```

The input can also be one XLSX or a directory of XLSX files. `--limit` is only for
focused trials. The full run has no worksheet-byte or cell-count cap. It uses the
existing byte/state-machine XML scanner, including hierarchy validation and its
explicit malformed/unsupported XML errors. No DOM, regex or cell index is built.
The scanner's structural limits (tag size/depth) still apply and are reported as
errors, never as absence of conditional formatting.

## Evidence and repeat analysis

- `sheets.jsonl`: one record for **every** worksheet XML entry, including hidden
  sheets. Records include workbook member name, numeric source ID, ZIP part,
  rule attributes/formulas/child names, classifications and error information.
- `sources/NNNNN.xlsx`: verbatim original XLSX for every workbook with CF or
  errors. This preserves namespace context, styles, names, extension XML and
  anything the initial metadata extractor did not understand.
- Raw CF nodes up to 16 KiB are inline as `raw_xml`; nodes up to 1 MiB are exact
  files under `nodes/`. Larger nodes remain in their retained workbook, addressed
  by exact uncompressed ZIP-entry byte offsets `[start,end)`. These thresholds
  choose evidence storage, not which nodes are analyzed.
- `workbooks.jsonl`: completed workbook outcomes and retained-source locations.
  `summary.json` is checkpointed every 25 workbooks and at completion. Its status
  distinguishes running, completed and completed-with-errors.

Recover any complete node without searching all Data.gov files again:

```powershell
python docs/visual-validation/experiments/cf-inventory/extract_node.py <inventory> --source-id 123 --part xl/worksheets/sheet1.xml --block 0 --output recovered.xml
```

Fragments retain original comments, CDATA, whitespace, prefixes and quoting.
They may need ancestor namespace declarations from the retained source to be
parsed as standalone documents. Failed sheets retain source evidence even when
a complete node span could not be established. No input workbook is modified.

## Classification and ranking

Only a narrow `cellIs` family with literal numeric/string operands is currently
eligible for a later invariance proof. Rule, range, priority, effective formatting
and cell inputs still need comparison against the optimized/recalculated states.
The inventory alone **never establishes unchanged rendering**. XML entities in
operands, unknown attributes/children, expression functions, aggregate rules and
unresolved namespace/extension cases remain explicit review/fallback candidates.
Formula summaries beyond 8 KiB are flagged truncated; raw evidence remains whole.

Nominal tile costs use a configurable fixed grid (`--rows-per-tile 256`,
`--columns-per-tile 32`). Whole-row/column and disjoint references are included.
The estimate is not Office.js pixel geometry and is not clipped to actual used
content. Overlaps are summed; rule-type/classification groups can overlap too.
Unknown ranges are counted separately, rather than assigned a misleading zero
capture cost. `report.py` ranks unsupported regions and includes concrete rules,
formulas and cached source links. It changes no capture-selection policy.

## Focused verification

`cargo test -p excel-shrink-cli cf_inventory` covers literal/reference distinctions,
extension rules, range costs, explicit malformed XML outcomes, and exact evidence
recovery across all three storage tiers. `cargo test -p excel-shrink-cli
streaming_xml` includes every chunk boundary of a node with comments and CDATA.
Filesystem tests may require normal permissions on Windows because the sandbox
can deny publishing renamed temporary summaries.
