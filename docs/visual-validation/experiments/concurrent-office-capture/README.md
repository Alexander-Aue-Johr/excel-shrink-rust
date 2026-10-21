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
