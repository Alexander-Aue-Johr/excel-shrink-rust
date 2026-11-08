# SheetShrink web entry acceptance

Two bounded tests exercised the ordinary SheetShrink configurator in the in-app
browser, native cleanup subprocess dispatch, retained output and the existing
Excel comparison engine. Each test used one workbook and one processing worker.
Enron continued with admission temporarily set to three groups and restored to
four afterward. No production host or runner was replaced.

The executable was built from application commit `9e6e2f2` and has SHA-256
`ed217706d680c13166d95cec589cfed81ff2ca11de8edb1215c9e10392934bc3`.
Private inputs, original reports, process ownership receipts and acceptance
results are retained under
`G:\deltasieve-work\sheetshrink-web-acceptance-20261108`.

| Case | Wall time | Cleanup outcome | Native outcome |
| --- | --- | --- | --- |
| Existing sparse-formatting fixture | 32.43 s | Byte-identical output retained | One Original process; redundant comparison input omitted with identity evidence |
| Synthetic unstyled blank cells | 38.61 s | 2,010 empty cells removed; seven numeric cells retained | Original and cleaned partner opened, saved and rebuilt in their existing processes |

For the second case, the input contains 2,017 cells and occupies 6,164 bytes.
The retained output contains seven cells and occupies 2,015 bytes. Independent
ElementTree parsing verifies the exact coordinate/value mapping on both sides.
The original fixture hash is unchanged. The report independently identifies
different input bytes and observed Original and cleaned-partner rebuild states.

Both runs completed as `completed_with_limitations`, with no failed pair and
zero capture candidates. No pixels were compared in these cases. Their evidence
establishes application orchestration, output retention and the identity/empty-
candidate paths. Detection accuracy requires the separate mutation and full-
reference experiments in the roadmap.

Browser inspection confirmed the selected producer, editable paths, progress,
opening-time/RAM displays and final outcome. Test tabs and hosts were closed.
The observed Excel PIDs, 29104 for the first test and 13384/52996 for the second,
were absent after completion. The subsequent Enron snapshot showed its runner
alive with 7,545 completed comparisons, four active groups and ten exceptions.

The inspection also exposed historical `Default` labels inside the shared
process panel despite the surrounding SheetShrink application branding. A
separate presentation change addresses those labels while preserving retained
lane IDs and raw report data. The narrow viewport also showed horizontal
overflow; responsive layout acceptance remains open.
