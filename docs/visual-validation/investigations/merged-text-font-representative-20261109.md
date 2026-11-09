# Merged text uses a covered empty cell's font

## Retained case

The Destatis workbook
`statistischer-bericht-personengesellschaften-gemeinschaften-5731101187005.xlsx`
has an observed Original-rebuilt versus SheetShrink-rebuilt difference in
`Informationen zur Statistik!A1:I10`. The changed text is in merged range
`A6:H6`. The original row has explicit height 177 and `customHeight="1"`.

Cell A6 has style 237, shared-string index 1081, font `MetaNormalLF-Roman` size
10, and justified wrapped alignment. B6:H6 are empty with style 238, using
Calibri size 11. That style has no explicit `applyFont` attribute. The cleanup
removed B6:H6 and changed row spans from `1:8` to `1:1`. Styles, shared strings,
column definitions, merge definitions, row height and the text cell's style and
value remained identical in the prepared packages.

The retained production image pair has 43,767 changed overlapping RGBA pixels
within rectangle `[2, 118, 897, 321)`, with identical 979 × 663 image sizes.
Visual inspection places the difference in the justified text paragraphs.
The source occurrence has study record ID `0eab18bfc2d093210f4a` and attempt
`targeted-60fd3ef2-eeb9-4621-9d74-099cd8e58fe3`.

## Native controls

Each control opens the same original and a separately prepared candidate,
rebuilds both in Excel and captures the independently supplied `A1:I10` area.
The package mutation changes only `xl/worksheets/sheet5.xml` relative to the
existing SheetShrink result. All variant ZIP CRC checks passed.

| Candidate | Changed overlapping RGBA pixels | Overall seconds |
| --- | ---: | ---: |
| Existing SheetShrink result | 43,767 | 19.106783 |
| Restore only original row spans | 43,767 | 19.0345682 |
| Restore only B6 and original spans | 0 | 19.0780842 |
| Restore only E6 and original spans | 0 | 19.505587 |
| Restore only H6 and original spans | 0 | 19.9521545 |
| Restore B6:H6 and original spans | 0 | 18.355006 |

These interventions reproduce the difference and isolate the required covered
cell information. They support retaining one local font representative instead
of restoring every removed cell. They do not establish a general typography
model for all merged layouts.

The unchanged-result receipt SHA-256 is
`1a1e5596cc13bf7efd6e16ee612a9d141dfa8fbcf8653cf3670a5377fab116dd`.
The B6-only receipt SHA-256 is
`4eb99a24d024f409079042be78fd92f1d5264fe426469801e835727e9ece0fa5`.
The span-only receipt SHA-256 is
`24ed221f92e0f98d0982eeaf8a728cc81dbe58291c43325fa2b7e1459e5eaf4e`.
Complete inputs, preparation identities, variant receipts and pixel measurements
remain under `G:\deltasieve-work\merged-font-control-20261109`.

## Regression coverage

`merged_text_keeps_one_local_font_even_without_apply_font` reproduces the
fixed-height, explicit-default-fill style without an applyFont flag. It requires
one retained local font representative and removes the repeats.
`merged_font_representatives_are_local_to_each_merge` checks that retaining a
font in one merged area does not suppress its representative in a second area.
The complete core unit suite passed with 295 tests and one ignored test.

## Normal-entry-point verification

The optimized `sheetshrink` executable processed the retained original through
its ordinary command-line entry point. It retained B6 with style 238 and removed
C6:H6, producing spans `1:2`. The new output was opened, rebuilt and compared
against the original by the independent native reference command. Both images
were 979 × 663 pixels and differed in zero RGBA pixels. This command completed
in 22.9386628 seconds.

Original size was 739,544 bytes. The old cleaned output was 571,496 bytes and
the corrected output 571,503 bytes. The corrected archive is seven bytes larger
than the previous cleaned output. The observed cleanup command took about
242 milliseconds in this single-file invocation. This is not a throughput
benchmark.

The fixed output SHA-256 is
`5692133a43d738f4d62e897aef60f84e549a722f31e06ef42751a13bf4fa2c77`.
The normal-entry native reference receipt SHA-256 is
`fd4dfd1423e78b870cfa21a50e88db2238c4c7088fdada49cbcd7417a0353cc0`.
The source and historical production verdicts remain unchanged. This reference
check covers the supplied affected area. A new production validation must still
inspect the changed output before replacing its historical corpus verdict.

The normal targeted-comparison workflow subsequently compared the corrected
output against the same original across all 264 regions selected for the file.
Its OR-versus-SDR result has zero pixel, geometry and rendering differences.
The report status is `completed_with_limitations`, retaining the ordinary
targeted-coverage qualification. Capture selection and transport took 107,227
milliseconds. This result is separate from the independent fixed-area control.
Its report SHA-256 is
`35136c9e05a8b6294a1abd1ba9980c06c3a65d755aeb236fafbb52f0c5d37a9d`.
The report is retained in `fixed-full-targeted-retry/report.json` beneath the
experiment root. The first invocation was rejected before Excel startup because
its requested Observatory group directory was absent. That failed invocation
remains separate. The historical study record has not been relabeled or replaced.
