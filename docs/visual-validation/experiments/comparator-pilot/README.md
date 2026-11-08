# Bounded external comparator pilot

## Status on 2026-11-08

No external comparator execution or throughput measurement has been completed.
A bounded local inventory found no matching Synkronizer, xlCompare, DiffEngineX
or Spreadsheet Compare uninstall entries. The standard Office16 DCF executable
locations and the standard xlCompare/DiffEngineX executable locations were also
absent. Portable or nonstandard installations remain possible. Installed
edition/version and licensing must be verified before any measured run.

## Documented capabilities

Microsoft describes workbook-wide cell comparisons and exportable results.
Availability depends on the Office edition. The documentation examined does not
establish a rendered-pixel oracle or a supported unattended report-export API.
[Microsoft workbook comparison](https://support.microsoft.com/en-us/excel/compare-two-versions-of-a-workbook-by-using-spreadsheet-compare)

Synkronizer 11 Developer documents file-pair and matching-folder comparisons,
saved reports and logs through `synk.exe`. Its command-line utility starts a
hidden Excel instance. Record the edition because command-line availability is
edition-dependent. This is documented capability, with no local timing evidence.
[Synkronizer command-line utility](https://www.synkronizer.com/usermanual/synkronizer-11/commandline_utility.htm)

xlCompare documents command-line file-pair comparison and exportable reports.
Opening its comparison interface from a command line does not by itself prove
unattended export or corpus-scale suitability. Verify those steps experimentally.
[xlCompare command-line parameters](https://xlcompare.com/excel-compare-command-line-parameters.html),
[report export](https://xlcompare.com/review-differences.html)

DiffEngineX documents unattended command-line comparisons and saved difference
reports. Preserve exact switches, report files and exit codes. Verify formatting
and dependent-display coverage separately from value/formula detection.
[DiffEngineX help](https://www.florencesoft.com/xldiffhelp.html)

These sources do not establish the tools' internal algorithms. No statement
about lazy comparison, pixel equality or acceleration is inferred from a UI.

## Frozen five-pair design

Use small synthetic fixtures with declared mutations and no private data.
Generate the baseline and mutation once, retain both SHA-256 hashes, and supply
the same bytes to every comparator. Do not alter inputs to accommodate one tool.

| Pair | Controlled change | Questions |
| --- | --- | --- |
| Identical | Byte-identical copies | Does a complete report correctly contain no changes? |
| Offscreen and hidden | Changed values far below the initial viewport and in a hidden row | Are both found without scrolling, and are hidden results distinguished from visible changes? |
| Direct presentation | Fill, border, row height and column width changes | Which properties appear in the report and which affect rendering? |
| Dependent presentation | Unchanged CF/chart definitions with changed precedent values | Are dependency-driven display changes represented, beyond reporting the edited input? |
| Sparse formatting | Small data table plus extensive empty formatted cells | Does the tool complete, and what work and report volume does it incur? |

For each pair, distinguish detection of an input edit from detection of its
dependent visual consequence. Hidden input changes are expected structural
differences even when no visible pixels change. An independent bounded Excel
capture supplies visual evidence for the selected visible regions.

## Execution record requirements

Record tool edition/version, executable hash, Windows/Excel versions, all
comparison options, input hashes, wall time, process-tree CPU and peak committed
memory, report byte size and machine-readable outcome where available. Run
serially with a fixed external deadline, preserve partial reports on failure,
and record required manual interactions. Do not count timeouts as equality.

First inspect whole-workbook report coverage without scrolling. Then establish
which tasks are genuinely comparable to DeltaSieve. A structural difference
report and a rendered-image report have different output semantics. Report both
coverage and cost rather than treating one output as an interchangeable oracle.
Retain unsupported properties and missing editions as explicit availability
outcomes. The pilot remains incomplete until executable observations exist.

## Reproducible fixture checkpoint

`fixtures.py` creates five controlled pairs as deterministic OOXML test packages.
It requires a fresh output directory and publishes `fixtures.json` with exact
input hashes and declared changes after producing all ten files. Generated
workbooks remain private test artifacts outside Git. Run the independent package
checks before sharing a frozen fixture set.

```powershell
python -m unittest discover -s docs/visual-validation/experiments/comparator-pilot -p test_fixtures.py -v
python docs/visual-validation/experiments/comparator-pilot/fixtures.py --output G:\deltasieve-work\comparator-pilot-20261108\fixtures
```

The package check parses every XML member, verifies repeatable hashes, confirms
that only the identical pair has equal whole-file hashes, and checks the input
values, hidden row, layout, sparse-format mutation and unchanged CF/chart
reference expressions. The dependent chart's numeric cache reflects the changed
input. Its formula and CF definition remain unchanged. The sparse case contains
20,000 explicitly formatted blank cells and seven numeric cells.

`accept-fixtures.ps1 -Root G:\deltasieve-work\comparator-pilot-20261108` opens the
ten inputs read-only in one owned Excel instance and retains native observations
and PID/creation-time ownership. Invoke it in a separate process with an external
deadline and working-drive TEMP/TMP, rather than in a shared interactive Excel
session. A 120-second external deadline was used for acceptance. Enron admission
was temporarily reduced to three groups, then restored to four in a finally
block. The owned Excel process was confirmed absent afterward.

The native acceptance on Excel 16.0.20527.20150 opened all ten inputs and retained
unchanged input hashes. It observed both offscreen values changing from 7 to 9,
row 3000 remaining hidden, row 1 changing from 15 to 30 points, different A1
fills, and one chart plus one CF rule in both dependent variants. Excel reported
column A widths of 11.29 and 23.29, illustrating that its effective width units
differ from the literal XML attributes 12 and 24. These observations establish
native input acceptance; independent rendered captures remain pending.

Private evidence is retained under
`G:\deltasieve-work\comparator-pilot-20261108`. The first native attempt failed
before opening any workbook because the helper environment did not expose
`Get-FileHash`. Its observations remain as `native-fixture-observations-attempt1.json`.
The corrected helper uses .NET SHA-256 and retains its successful observations
in `native-fixture-observations.json`. This was a helper failure and provides no
evidence against workbook validity.

## External acquisition checkpoint

The official DiffEngineX download page offers a no-installer ZIP for its 30-day
evaluation. xlCompare documents a portable option and a fully functional download
without registration. Their license agreements attach conditions to acquisition
or use, so explicit trial-license approval was requested before downloading or
executing either product. No external execution result is claimed by the native
fixture acceptance above.
[DiffEngineX downloads](https://www.diffenginex.com/excel-differences-download.html),
[DiffEngineX terms](https://www.florencesoft.com/xldifflicense.html),
[xlCompare downloads](https://xlcompare.com/download.html),
[xlCompare terms](https://xlcompare.com/copyright.html)
