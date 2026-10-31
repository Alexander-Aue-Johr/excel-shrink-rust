# Targeted comparison WIP

`excel-shrink-cli regression targeted-compare` compares one original XLSX with one
Default shrink XLSX without starting the corpus runner or Observatory. It copies
both inputs, keeps each copy open in a private Excel instance, and writes PNGs,
`report.json`, and `telemetry.json` to a new output directory. No blanket sheet
capture is made.

```powershell
excel-shrink-cli.exe regression targeted-compare `
  --original C:\path\original.xlsx `
  --default C:\path\default.xlsx `
  --output C:\path\new-output-directory `
  --host-script C:\path\excel-shrink-cli\office-capture\targeted-comparison-host.ps1
```

`--observatory-group-dir` can point to an existing `groups/<id>` directory. The
command then writes `targeted-comparison.json` there for the Observatory group
view. This is an explicit option; the standalone command never starts a run.

## Implemented path

1. Compare cell XML in matching worksheets. Empty styled-cell changes collapse
   to one row geometry probe instead of thousands of screenshots.
2. Open Original (`O`) and Shrink Default (`SD`), save both, and add cell changes
   produced by the ordinary Excel save.
3. Capture each candidate with one neighboring cell on every side. Read its
   rendered position and size. Rebuild and capture SD, then rebuild and capture O.
4. Compare PNG pixels and preserve all images and geometry in the report. The
   `also_changed_by_original_recalc` field identifies recalc drift in O.

The current code is an experiment, **not complete visual validation**. It
stops after 64 changed-cell candidates. It does not yet discover chart, drawing,
style, row, column, or all formula-dependent changes. It does not detect new
candidate cells introduced only by recalculation. Empty-cell changes are
mapped to column A in the same row; this catches row-height effects but does
not prove every visible consequence. Reopening after save is not checked.
The report records these limits and Check Performance as
`unavailable_not_tested`; it must not be read as a clean comparison of the
whole workbook.

## Local integration check

On 2026-10-31 the standalone Windows executable compared the synthetic
`baseline.xlsx` and `removed-empty-cell.xlsx` from the row-top experiment.
It selected one candidate, `Rows!A500000`, and captured
`A499999:B500001` in four states. The original and changed copy differed
before recalculation and after rebuilding the changed copy; rebuilding the
original did not change its captured region. The original row height was
53.25 points in both captures. This verifies the basic control sequence and
the collapse of 16,384 empty-cell removals to one row probe. It does not test
Check Performance, drawings, or production corpus coverage.

## Next machine: Check Performance

The machine used here has no Check Performance installation. On the machine
that has it, add a third persistent workbook lane (`CP`) around the existing
comparison phases. First measure what ordinary Excel open/save does to O,
then run Check Performance on a separate O copy, save it, and discover its XML
changes. Capture CP regions before and after its rebuild, keeping O open for
matching control images. Label changes also made by ordinary O save or O
rebuild separately. Test the real Windows automation command there; do not
claim CP success from the present placeholder status. Keep the top-level Rust
procedure readable and keep this WIP commit as the integration base.

Before using this for a corpus, replace the in-memory cell map with a bounded
streaming comparison, add complete candidate discovery and report filters, and
connect actual Office.js heartbeat/progress to the Observatory cards. The
current cards display phase telemetry but no add-in heartbeat because this
experimental host uses COM screenshots.
