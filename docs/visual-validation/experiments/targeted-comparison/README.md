# Targeted comparison workflow

The Observatory's **Targeted recheck** starts this bounded comparison on private
copies. It does not run or change the corpus queue. The same entry point is
`excel-shrink-cli regression targeted-compare` through
`office-capture/run-targeted-comparison.ps1`; a private desktop is required.
`--skip-check-performance` explicitly omits CP. `--operation-timeout-seconds`
bounds commands; closing remains capped at 15 seconds.

## Retained states and ordering

Original (`O`), Default (`SD`) and the optimized Check Performance copy (`CP`)
stay open without an explicit rebuild. Rebuilds use separate copies (`OR`,
`SDR`, `CPR`), so newly discovered candidates can still be captured in every
pre-state. There are at most six owned Excel instances, or four without CP.
This is necessary evidence preservation, not duplicate timing measurements.

1. Preserve raw inputs and normal opening/saving controls for Original and Default.
2. Make CP from saved Original. Preserve CP's additional opening/saving control,
   then its own before/after Optimize all XML. Require positive native completion.
3. Select XML candidates, compare actual relevant heights, and capture direct
   Original/Default/CP evidence before any explicit rebuild.
4. Rebuild Default and CP copies. Compare XML before/after each rebuild/save,
   discover new candidates, and capture missing regions in all retained states.
5. Finish comparisons against Original-before. Only then open/rebuild Original's
   copy. Search again and backfill every newly required partner image.
6. Compare Original-before/after, both-rebuilt pairs, and each variant's own
   before/after states. Retain every evidence image and verify all owned exits.

`staged.rs::Workflow::execute` is the domain-level reading order. `host.rs` owns
bounded command transport; `TargetedResident.psm1` reuses the native resident
launcher and command protocol. Captures, save, rebuild, geometry and row-height
checks use **Office.js in those same live workbooks**, without clipboard capture.
Only the verified native Check Performance trigger borrows the exact owned Excel
application through COM. It never uses a global active-object lookup or global
pointer input. Each exported saved copy removes only its verified injected
add-in; the open workbook keeps its add-in.

## Candidate and height selection

`xml_changes.rs` streams worksheet byte XML into fixed-size cell fingerprints
on temporary disk files, retaining relevant row attributes, columns and sheet
defaults. Cloned states share immutable fingerprint files and row metadata. Ordinary saves and rebuild-copy reopening have
separate transitions. Full before/after content fingerprints plus style/string
context identify exact overlaps with Original opening/saving changes. The report
never infers equal changes from matching addresses alone. This is conservative
XML attribution, not exhaustive semantic equivalence across rewritten styles.

`heights.rs` excludes a row from automatic-height measurement only if it exists
on both sides with equal explicit fixed height and unchanged hidden status.
Deleted rows and changed definitions remain eligible. Column/default/context
changes can select the full Excel height, including empty rows below content.
The existing Office.js height scanner measures the remaining spans. Cached runs
are reused only within the same immutable workbook state and exact span request.
Coverage is validated before comparison; missing partner heights are an error.

Every changed height run is reported over its full extent. Images are selected
at its boundaries and the next row to expose displacement. **Those images are
representatives, not a visual proof of every row in the run.** Cell candidates
and padded image neighborhoods remain independently selected.

The height command has a 30-second in-add-in budget between batches and the
outer command deadline. Budget exhaustion fails with incomplete coverage; it
never certifies equality or silently starts a full-workbook capture.

## Reports and limits

`report.json` preserves the existing Default/CP findings and adds:

- `transitions`: actual source/saved files and content/context fingerprints,
  with exact overlaps marked `also_in_original_open_save`.
- `height_checks`: requested spans, excluded fixed rows, changed height runs and
  explicit completeness.
- `comparisons`: direct, variant-rebuilt-versus-Original-before, each state's own
  rebuild-copy drift, and both-rebuilt pairs with PNG paths and geometry flags.

`partial-report.json` retains available comparisons during execution and partial
outcomes on failure. `failure.json` and final telemetry distinguish unsuccessful
work from unconfirmed process exit. The Observatory's process samples remain
labelled PID liveness, not fabricated Office.js heartbeats.

There is no candidate-count, worksheet-byte or cell-count rejection limit. Cells are merged from ordered disk streams instead
of an in-memory cell index; style/string context is hashed in chunks. Scratch
files are released with their snapshots, including on failure. Memory still
scales with layout metadata and actual differences. Drawing/chart/conditional-formatting and
all style/layout dependencies are not exhaustively selected. Matching selected
images is not whole-workbook visual equality. Reopening a rebuild copy can have
its own effects: these XML changes are retained separately and a visual
before/after-copy comparison does not by itself prove exclusive recalculation
causality. Opening can also recalculate when Excel requires it.

## Focused verification

`run-smoke.ps1 -Output <fresh-directory>` runs actual private Excel clients:

- `-CheckPerformance`: identical input pair, native optimization, new candidates,
  six retained states, actual images and confirmed exits.
- `-RecalcCandidate`: removes a source value while retaining a formula cache;
  explicit workbook calculation settings keep the indirect change for rebuild.
  The verifier requires the late formula candidate and all pre-state images.
- `-SaveOverlap`: a cell removed by both Default and ordinary Original save must
  be recognized as an exact overlap.
- `-CleanWorkbook`: verifies positive "No optimization needed".
- `-FailOptimization` / `-FailCapture`: deliberately fail after real startup,
  retaining partial results and checking every owned process exits.

The browser harness `test-panel.cjs` verifies real shaders, stable canvases, the
dynamic state/phase matrix, history and fullscreen. `test-host.cjs` checks the
actual HTTP launch and an explicitly bounded silent-host failure. Existing
results are historical records; this workflow does not upgrade or overwrite them.
