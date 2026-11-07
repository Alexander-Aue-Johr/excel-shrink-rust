# COM conversion on shared and private desktops

This probe runs four production legacy converters together in each trial.
The four arrangements use shared/private/private/shared ordering. An independent
owned job closes only experiment processes. Private desktop handles are retained
until those processes exit. The source is the small normal.xls fixture from the
legacy-conversion-errors experiment, with one text cell and one numeric cell.
No corpus is stopped or reconfigured by this experiment.

All sixteen conversions succeeded. The helper and Excel window desktop names
matched in every case, including every distinct private desktop. Styles, shared
strings, theme and sheet XML matched after removing only the worksheet's
Excel-generated revision UUID. Raw worksheet byte equality is therefore false.
The user's input desktop remained unchanged.

| Trial | Arrangement | Four conversions, wall seconds |
| --- | --- | --- |
| 1 | Shared desktop | 24.203 |
| 2 | Separate private desktops | 36.860 |
| 3 | Separate private desktops | 25.085 |
| 4 | Shared desktop | 26.448 |

**The throughput comparison is inconclusive.** During the experiment, the corpus
preparation control changed from four processes to sixteen and its shrink thread
count changed to one. That load change was not made by the experiment script.
The tiny fixture also measures startup/configuration overhead rather than large
workbook loading. These timings do not justify changing the corpus converter's
desktop arrangement. The working desktop-placement mechanism is recorded for
future controlled tests. Existing native capture launchers remain unchanged.

```powershell
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File `
  docs/visual-validation/experiments/legacy-conversion-desktops/run.ps1 `
  -Output V:\validation-experiments\conversion-desktops `
  -Fixture V:\validation-experiments\legacy-conversion\normal.xls
```

Choose a new output directory. Keep competing workload and preparation controls
fixed when comparing throughput. Each trial has a 90-second outer deadline.
