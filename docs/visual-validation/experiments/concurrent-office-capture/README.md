# Concurrent worksheet capture

`benchmark.py` runs the same workbook twice, first with one worker and then with
two workers, alternating the order on the second repetition. It requires the
capture launcher, an explicit workbook and capture configuration, and an output
directory. Capture outputs and timings are written there.

```text
python benchmark.py --launcher <run.ps1> --workbook <workbook.xlsx> --config <config.json> --output <output> --repeats 2
```

## Laptop trial, 2026-10-21

The synthetic workbook had 1,000 rows and 12 numeric columns. Capture used a
32-megapixel limit and an inactive Windows desktop for each independent Excel
instance.

| Workers | Two captures | Outcome |
| --- | --- | --- |
| 1 | 28.55 seconds | Both completed: 14.20 and 14.35 seconds |
| 2 | — | One completed; the other add-in exceeded the 120-second startup limit |

The trial stopped at the failed concurrent run. It does not establish a
throughput improvement. Keep corpus runs serial until concurrent add-in startup
is reliable on the target machine. The normal paired comparison keeps its two
workbooks together on one inactive desktop; these independent sessions each
used their own desktop.

## Repeat after capture runtime fixes, 2026-10-21

The same 1,000-row, 12-column workbook was captured twice per condition with
the launcher from main commit `2ff144e`. One serial and one parallel condition
were run; there was no further repetition.

| Workers | Two captures | Outcome |
| --- | --- | --- |
| 1 | 18.13 seconds | Both completed: 9.37 and 8.76 seconds |
| 2 | Incomplete | One completed; the other add-in exceeded the 120-second startup limit |

The launcher reported `Office add-in did not start before the startup timeout.`
Its cleanup completed; a subsequent Windows process query found no remaining
Excel, capture host, or PowerShell launcher belonging to the captures.

This measures independent single-workbook captures, not two complete
original-versus-shrunk comparisons. The parallel condition did not produce a
valid throughput measurement. Corpus comparisons therefore remain serial.
