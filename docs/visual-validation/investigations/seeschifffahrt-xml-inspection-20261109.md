# Seeschifffahrt XML inspection, 2026-11-09

The user identified `statistischer-bericht-seeschifffahrt-2080500241065.xlsx`
during a long saved-XML inspection. The retained record
`3f7522d81f73fce313a4` completed with status `unchanged` under the documented
targeted coverage. Overall comparison time was 232.219 seconds. The retained
attempt is `targeted-14b3d1f6-1005-44a2-b5f4-9821ce2f6b9d`.

## Measured work

The snapshot cache reported six parsed versions, fourteen reused versions and
38.391 seconds of parsing time. The operation telemetry recorded the following
aggregate request times. They can overlap and must not be added as a complete
wall-clock decomposition.

| Operation | Calls | Seconds |
| --- | ---: | ---: |
| Open batch | 1 | 14.897 |
| Save | 2 | 5.859 |
| Recalculate | 2 | 4.555 |
| Heights | 18 | 0.915 |
| Measure batch | 18 | 0.617 |
| Capture many | 18 | 2.386 |
| Close | 2 | 5.843 |

The report measured capture selection and transport at 1.748 seconds and image
comparison at 0.140 seconds. Nine candidate regions remained after planning.

## Large control evidence

| Transition | XML change records | Gzip bytes |
| --- | ---: | ---: |
| Original opening and saving | 3,533,165 | 282,716,862 |
| SheetShrink input against original input | 2,637 | 103,400 |
| SheetShrink opening and saving | 3,532,721 | 282,800,059 |
| Original in-place rebuild and save | 0 | 119 |
| SheetShrink in-place rebuild and save | 0 | 119 |

The two save controls alone retain about 539 MiB of compressed audit evidence.
Their changes are control observations, not millions of SheetShrink findings.
The source `staged::audit` computes content changes, builds the original-save
attribution keys, annotates changes and writes the complete gzip evidence.
Those steps lack individual timings in this attempt. The existing measurements
therefore do not establish the exact cause of the remaining elapsed time.
Add explicit audit-stage timings before attributing it to parsing, attribution,
compression, storage or scheduling. Preserve the retained evidence and the
separate control-versus-validation selection behavior.

## Why nine capture regions remain

The 3.53-million counts describe each input's own Excel open/save transition.
They are attribution controls. `staged::audit` retains their evidence without
adding every control change to the requested visual regions. The final selector
instead compares the Original-rebuild and SheetShrink-rebuild states, resolves
actual styles through `StyleIdentity`, adds relevant layout, conditional-format
and chart dependencies, then intersects and merges candidate areas under the
documented content bounds. Unknown supportedness retains fallback coverage.
This is not a demonstrated reduction of 3.53 million independent visual defects
to nine screenshots.

The input Original-versus-SheetShrink audit contains 2,469 cell records, 159 row
records and nine column records, totaling 2,637. The cleanup receipt attributes
102,191 removed items to empty cells without content, nine to redundant column
range merges, one to sheet-data tail pruning and three to trailing layout-only
rows. Thus the retained change count is neither a removed-cell count nor a
pixel-difference count. It excludes exterior formatting-only cell changes.

A complete count of the retained gzip records assigns 3,507,614 entries in each
save control to the six `csv-46331-*` sheets. The input comparison has only one
column record per CSV sheet and no cell records there. The final nine selected
regions cover `46331-01` through `46331-06`, `Erläuterung zu CSV-Tabellen`,
`Impressum` and `Informationen zur Statistik`. Large initial candidate sets on
the explanatory sheets are clipped and merged into one region per sheet.
The result remains a targeted validation with documented limitations.

## Style IDs and actual properties

Production cell, row and column comparison already resolves style-table indices
to properties. Pure permutations are covered by regression tests. Consequently
the millions of save-control records must not be explained as merely different
style numbers without examining their definitions.

The original and a separately retained pre-optimization Excel save of the same
source were available in Check Performance preparation `input-000977`. This is
an auxiliary control, not the deleted exact save artifact from the visual
attempt. Its baseline SHA-256 is
`3321086a17f596ace6c79a1a94239165a3783e54459f6383431772ffce6b9d06`.
The auxiliary `original-saved.xlsx` SHA-256 is
`e0eb2421b4eebfa99a97f42810584c62ed29ed236c0830f3ef98fdcde586c843`.

In `sheet13.xml`, cell A1 changes style 36 to 32 while retaining shared-string
index 192. B1 changes 64 to 52 while retaining shared-string index 193. The XF
table shrinks from 173 to 143 entries. Inspecting the definitions shows actual
property changes as well. Style 36 has number format 49, top alignment and
wrapping. Saved style 32 has number format 0 and left alignment. Style 64 has
number format 167 and border 2. Saved style 52 has number format 49, border 0
and indentation 1. These examples establish that this auxiliary save does more
than renumber styles. They do not classify every record in the visual attempt.

The separation of save controls from the rebuilt comparison is therefore
material to this case. Both sides can undergo Excel normalization without
making that normalization a SheetShrink regression. The case should be used as
an explanatory case study, not as an independent coverage or recall estimate.
The cleaned ZIP was also larger than its input, 15,022,001 against 13,534,365
bytes, despite about 2 MiB less uncompressed worksheet XML. This case does not
demonstrate an archive-size reduction.
