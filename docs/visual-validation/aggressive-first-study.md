# Aggressive-first study experiment

This branch prioritizes finding regressions before completing the full profile
matrix. It does not infer that Default or Maximum is visually identical merely
because Too Aggressive matched.

Start a saved assignment with:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\start-corpus-terminal.ps1 -Corpus Enron -Partition 2/2 -Order size-desc -AggressiveFirst -SkipBuild
```

Build the intended version before using `-SkipBuild`. The direct CLI equivalent
adds `--aggressive-first` to a resident study. The flag selects the autonomous
hash-sweep transport automatically. Runs without this flag keep the regular
comparison strategy.

## Global ordering

1. Prepare the selected workbooks, reusing compatible preparation bundles.
2. For every eligible workbook, validate Too Aggressive with the existing
   recalculation policy and capture Original versus Too Aggressive. Default and
   Maximum are not opened in advance.
3. When this primary comparison has a visual difference, open Default and
   Maximum and capture the aggressive mismatch regions. These are targeted
   raw-input screenshots, not complete profile or recalculation checks. Their
   full recalculation validation is deferred to the second global pass.
4. Run bounded diagnostic controls on the open temporary copies. Keep original
   evidence unchanged. Continue to the next workbook.
5. Only after screening the entire selected assignment, run the independent
   Default and Maximum comparisons, including the existing recalculation
   validation. Keep completed aggressive evidence. Retry aggressive profiles
   whose screening did not produce a complete result.

The scope selected by `--limit` applies to both passes. All selected corpora are
screened before starting the secondary pass. Historical partial captures are
queued again, rather than treated as complete independent validation.
This includes historical row-height policies that compared pixels only on sheets
with equal heights; their completed-policy flag alone is not full pixel coverage.

A stop and later restart preserve both passes. `priority_validation.primary`
records whether the aggressive check completed; `priority_validation.secondary`
records whether the full profile matrix completed. A failed screening attempt is
not a visual-equality claim and remains visible. Prepared ZIPs remain available.
A compatible preparation build approval does not certify old visual results as
new measurements.

## Result states

- `screened`: aggressive profile checked, no reported difference in that check;
  Default and Maximum full checks remain pending.
- `screening-differences`: primary screening reported a difference; targeted
  follow-up and diagnostic evidence may be available.
- `screening-failed`: the primary check was not completed. The reason remains in
  the record; the later pass may retry it.
- `validation-incomplete`: the secondary pass did not establish full coverage.
- `unchanged` / `differences`: normal completed full-matrix results.

Targeted results carry `targeted_checks_complete=true`, `checks_complete=false`
and explicit checked ranges. They cannot increment the full-validation count.
The Observatory displays the active pass and separate screening/full counts.

## Excel process pipeline

The control room admits one additional comparison group during aggressive-first
runs. That group can finish stored-value checks, isolated recalculations and
capture planning while the configured number of other groups capture images.
It waits for a capture-group slot before opening its resident workbooks. As soon
as a capture group exits, the prepared group can begin opening its workbooks.

The **Excel process limit** in the Observatory defaults to 8 (range 4–32) and
applies to owned recalculation, conversion and capture Excel processes together.
Capture groups reserve their required slots together before opening any workbook,
so two groups cannot each hold too few slots to finish. Waiting workers do not
create an extra Excel process. Lowering the limit affects new starts, while
already open processes finish normally. A group target of zero still drains
groups that were already admitted, including the preparing group.

## Capture and replay

See [the autonomous protocol](aggressive-first-capture-protocol.md). Workers
receive the full negotiated capture plan, hash each PNG and release its encoded
buffer before proceeding. Peers need not exchange one command per tile. The host
collects the complete hash manifests before requesting mismatch evidence.

The live `capture_context_tiles` setting defaults to zero: capture only mismatch
bands. Increasing it adds touching bands above and below. It is sampled at the
start of a sweep. The current geometry is full-width row bands, not a 2D grid.

The recaptured evidence must reproduce its offered digest and geometry. Drift
is reported as unstable, not as evidence of a shrinking regression. Equal tiles
have no stored preview unless requested by another evidence operation. Full
sweeps bypass edge-only and deferred CRC sampling; hard capture limits still
produce explicit incomplete checks.

## Display indicators

Capture copies already carry worksheet-level ignored-error flags. Additionally,
launchers disable `ErrorCheckingOptions.BackgroundChecking` through the exact
owned Excel process and verify it before releasing capture commands. The native
binding verifies the process behind the Excel window; it does not attach to an
arbitrary active application. The previous value is restored during cleanup.
Actual error values and formulas remain part of the workbook and comparison.

## Diagnostic controls

Primary images are frozen before mutations. The controller first checks that
replay reproduces the images, then performs two paired recalculations to detect
observed instability or stale-cache effects. This does not prove that every
unchanged workbook is free of volatile or external dependencies.

Remaining differences receive cumulative controls in this order:

1. Restore original row heights through the affected tile prefix.
2. Restore deleted whitespace values on affected sheets.
3. Restore deleted hidden content on affected sheets and recalculate.
4. Restore deleted hidden content on other sheets and recalculate.
5. Restore remaining deleted whitespace across sheets and recalculate.

Only actually deleted cells are restored. Formulas, typed constants and exact
Unicode strings remain distinct. Sources are read from prepared inputs; edits
apply to temporary open copies. Repair attempts record the requested/applied
mutations and before/after image evidence. An unsupported restoration or exhausted
budget is unresolved, never a successful explanation.

One effective repair can explain a difference within the checked regions.
Resolution after several retained repairs is labeled `combined`; it does not
claim that one individual change was the sole cause. Other categories include
`row_height`, `local_whitespace`, `local_hidden_content`,
`cross_sheet_hidden_content`, `cross_sheet_whitespace`, `recalculation`,
`unstable` and `unresolved`.

The Observatory exposes these categories as overlapping workbook filters.
Default differences not classified as unstable or resolved by recalculation are
also listed under **Default: investigate difference**. These are investigation
candidates, not automatically confirmed shrinker bugs. Empty formatting cells,
center-across-selection and row/column style inheritance require separate
formatting counterfactuals; the value-restoration cascade does not explain them.

Optional diagnostic failure cannot erase completed primary evidence. The parent
still confirms owned-process cleanup before removing temporary work directories.

## Focused tests

The branch includes protocol tests for autonomous sweeps and replay drift,
strategy tests for deferred versus full completion, typed-cell restoration tests,
marker-startup guards, and an opt-in real Excel diagnosis fixture. For the latter,
set `EXCEL_SHRINK_DIAGNOSIS_FIXTURE` to a directory containing `original.xlsx`,
`variant.xlsx` and `expected.txt`, then run:

```powershell
cargo test -p excel-shrink-cli experimental_diagnosis_excel_fixture -- --ignored --nocapture --test-threads=1
```

Build the release executable first: that test uses it for the HTTP capture host.
Run actual Excel integration tests serially to keep their render-state and
startup measurements interpretable.
