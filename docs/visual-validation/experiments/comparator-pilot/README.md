# Bounded external comparator pilot

## Status on 2026-11-08

The five-pair and two-large-pair xlCompare and DiffEngineX CLI trials have
completed. Controlled cost measurements, independent rendered reference captures and option/edition coverage
remain open. Before acquiring the trials, a bounded local inventory found no
matching Synkronizer, xlCompare, DiffEngineX
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
outcomes. Broader conclusions require the large-input and rendering follow-ups
below.

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

## Executed five-pair CLI observations

The user explicitly approved both trial agreements before acquisition and use.
The official portable DiffEngineX ZIP and xlCompare MSI were downloaded to G:.
The MSI was administratively extracted into a private directory, without an
ordinary product installation. Both executable Authenticode checks were valid.
The observed file versions were DiffEngineX 3.18.0.0 and xlCompare 12.5.0.0
(marketed as 12.05).

| Executable | SHA-256 |
| --- | --- |
| xlCompare | `4b3a8f9a740e73a60832f9411ff7d9e42b577ac368bc6356815ef25e8a93496b` |
| DiffEngineX | `4d23aa73dafc2ae9a7bee09563310cc15bd1d05f0f685d52d1e93de7b231220c` |

`run_pilot.py` verifies each frozen input and supplies fresh exact copies to the
documented report command. It retains stdout/stderr, exact arguments, process
metrics, full XLSX reports and a bounded text preview. DiffEngineX's positive
difference-count exit codes are distinguished from errors. Missing, unreadable
or timed-out reports stop further trials. The helper never interprets a report
as rendered-pixel equality. All ten measured trial input pairs remained
byte-identical after their comparisons.

| Pair | xlCompare seconds | DiffEngineX seconds | Observed exported content |
| --- | --- | --- | --- |
| Identical | 0.778 | 4.805 | Empty xlCompare report; DiffEngineX zero differences |
| Offscreen/hidden | 0.759 | 12.392 | Both export B2000; neither tested export lists B3000 |
| Direct presentation | 1.245 | 14.851 | Empty xlCompare report; DiffEngineX zero cell differences |
| Dependent presentation | 1.301 | 5.424 | Both export the B1 input edit; no separate dependent CF/chart finding |
| Sparse formatting | 2.022 | 13.958 | Empty xlCompare report; DiffEngineX zero cell differences |

The xlCompare trial export contains a message limiting output to its first ten
rows. Its effect on reporting must remain disclosed. Options/editions beyond
the exact retained commands were not exhaustively evaluated. The absence of a
format or hidden-cell entry in these reports does not establish absence of that
feature in the full products. The offscreen result directly establishes report
generation beyond the initial viewport for these commands.

These are single-trial wall times alongside Enron, preceded by one identical-pair
smoke test for each product. Resource receipts cover only the direct comparator
process and exclude any separate Excel process. They cannot support a fair
whole-process memory comparison between standalone and Excel-backed products.
The full receipts are under the private experiment's `xlcompare-five-pairs` and
`diffenginex-five-pairs` directories. Each Excel-backed test window restored
Enron admission to four groups afterward. No comparator process remained after
the pilot.

## Large-input follow-up

The user requested preserving limited trial uses for large files. DiffEngineX's
official home page specifies 30 days and at most 60 comparisons. Six invocations
have been used here before the large-input stage, including the smoke test.
No trial counter or license limitation is modified. Large-input follow-up takes
priority over further small repetitions. [DiffEngineX evaluation limits](https://diffenginex.com/)

`prepare_large.py` extracts two preselected Destatis members, verifies their
previously frozen SHA-256 identities and runs SheetShrink Default with one Rayon
thread. The pinned release binary is commit `317200b`; its Core has no diff from
the current checkout. These input pairs are prepared independently of external
trial invocations. Their visual outcome remains for independent validation.

| Input | Original bytes | Sheet XML bytes | Sheets | Default output bytes |
| --- | --- | --- | --- | --- |
| Kernhaushalt Gemeinden | 89,367,137 | 953,486,690 | 107 | 4,602,649 |
| Verkehrsunfälle Zeitreihen | 37,207,883 | 774,344,761 | 215 | 2,608,693 |

The SheetShrink preparation wall times were 19.323 and 9.801 seconds in one
concurrent-load trial. Shrinking and comparing workbooks are separate tasks.
Structural comparison costs are compared with DeltaSieve only where output
semantics and tested options are stated. Check Performance remains the cleaning
baseline. A trial/export limit must never become a whole-workbook equality claim.
## Executed large-input observations

Each large pair was compared once per product with a 600-second external
deadline. Every invocation completed with exit code zero and a readable report.
All trial input copies retained their exact frozen bytes. Enron continued at
three admitted groups during each large comparison window and returned to four
afterward. These concurrent-load observations remain exploratory.

| Input | xlCompare seconds | DiffEngineX seconds | xlCompare report bytes | DiffEngineX report bytes |
| --- | --- | --- | --- | --- |
| Kernhaushalt Gemeinden | 53.748 | 131.692 | 4,889 | 233,639 |
| Verkehrsunfälle Zeitreihen | 21.711 | 161.221 | 4,890 | 460,198 |

The xlCompare reports are empty single-sheet workbooks, its documented output
for no differences with this command. Its observed direct-process peak working
sets were 6.908 and 2.808 GiB. These cannot be compared with DiffEngineX's
direct-process memory counters because the latter separately automates Excel.
The pilot did not measure that companion Excel process's full lifetime resources.

The DiffEngineX reports contain 108 and 216 worksheets respectively, comprising
one report per source sheet and a comparison summary. Independent package-name
matching confirms every source worksheet is represented. Each summary has the
expected 107 or 215 rows and reports zero cell differences for the tested
formula-comparison configuration. This provides a structural report outcome,
without certifying rendered equivalence or dependent presentation coverage.

The first analysis draft erroneously omitted 21 numeric-typed summary names in
the Verkehrsunfälle report. Inspection showed that labels such as `1.1` and `11`
were stored numerically by the report writer. The corrected analysis counts all
summary rows and independently matches actual report worksheet names to source
names. It verifies all 215 sheets. Both analysis versions are retained; the
final receipt is `large-input-acceptance-v2.json` in the private experiment root.

The experiment has issued eight comparisons per product, including small smoke
tests and both large cases. It does not assume how many trial uses may have
preceded this experiment on the machine. No further small repeated invocations
are planned ahead of large-input work. Native input acceptance, full reports,
process receipts and hashes remain under `xlcompare-large-pairs` and
`diffenginex-large-pairs`. The subsequent Enron snapshot confirmed 7,442 completed
comparisons, four active groups, 9,347 remaining and ten skips. Independent native
visual reference checks and a matched DeltaSieve cost evaluation remain open.
