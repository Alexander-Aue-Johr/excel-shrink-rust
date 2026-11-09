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
