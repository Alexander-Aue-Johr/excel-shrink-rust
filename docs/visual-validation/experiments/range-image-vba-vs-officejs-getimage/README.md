# Range image benchmark: VBA CopyPicture vs Office.js getImage

## Goal

This is an engineering experiment for the Excel Shrink visual-regression pipeline.
It is not a paper experiment.

The drawing-bounds benchmark showed that Office.js does not offer a compelling
engineering advantage for geometry collection once startup/lifecycle complexity is
taken into account. This experiment tests the one Office.js API that could still
justify that complexity: `Excel.Range.getImage()`.

The question is deliberately practical:

> Is `Range.getImage()` enough faster than the VBA screenshot path to justify an
> Office.js dependency in the visual-validation pipeline?

## Compared paths

### VBA

For every planned capture:

```text
Range.CopyPicture(xlScreen, xlBitmap)
-> temporary ChartObject
-> Chart.Paste
-> Chart.Export(..., "PNG")
-> PNG file available locally
```

The timer starts immediately before `CopyPicture` and stops after `Chart.Export`
returns. Workbook open/close, plan generation, result serialization, and temporary
chart cleanup are excluded.

### Office.js

For every planned capture:

```text
Range.getImage()
-> context.sync()
-> Base64 PNG
-> POST to the local experiment host
-> decode and persist PNG
-> PNG file available locally
```

`Range.getImage()` itself is available from ExcelApi 1.7. This unattended desktop
benchmark requires ExcelApi 1.11 because the task pane also closes each benchmark
workbook after submitting its result. The experiment records rendering and
transfer/persistence separately and reports their combined capture time. Workbook
open/close, task-pane startup, sideload registration, web-host startup, and plan
generation are excluded.

Office.js ranges are queued in configurable batches before each `context.sync()` so
the benchmark does not deliberately introduce one round trip per range.

## Capture plan

This benchmark is about the PNG-producing operation, not about rediscovering the
production screenshot bounds. A deterministic capture plan is therefore generated
once and consumed by both implementations.

By default it selects up to three visible worksheets per workbook:

1. the first visible sheet;
2. the visible sheet with the most shapes;
3. the visible sheet with the largest `UsedRange` area;
4. additional visible sheets in workbook order only if needed to reach the requested
   count.

Duplicate selections are removed. Use `-AllVisibleSheets` to benchmark every visible
worksheet.

Sheet selection still uses the real `UsedRange`, but the capture address is bounded
to the top-left part of that range (500 rows x 64 columns by default). This avoids
stale formatting that makes `UsedRange` span whole rows, whole columns, or nearly an
entire worksheet and causes screenshot APIs to fail for reasons unrelated to the
implementation being compared. Use `-MaxCaptureRows` and `-MaxCaptureColumns` to
change the cap. This remains a benchmarking range, not the final Excel Shrink
screenshot-bound policy.

## Correctness diagnostics

Both producers persist their PNGs below `.tmp/visual-validation/` so the artifacts
can be inspected directly. The summarizer compares:

- planned capture IDs;
- missing/error captures;
- PNG width and height;
- PNG file sizes;
- SHA-256 byte equality.

Byte equality is only a diagnostic. Different Excel rendering paths can legitimately
encode PNGs differently, and equal dimensions are not proof of pixel equality. Do
not use this experiment as a visual-correctness gate without inspecting
representative images or adding a real pixel decoder/comparator.

In particular, this experiment should reveal whether the two APIs include floating
Excel drawings in the same way for the selected ranges. Do not assume that from the
API name alone.

## Output

Repository results:

```text
results/
  files.txt
  capture-plan.tsv
  vba-copypicture.json
  officejs-getimage.json
  officejs-getimage.partial.json   # only after a partial Office.js failure
  summary.json
```

PNG artifacts are intentionally outside the repository result directory:

```text
<repo>/.tmp/visual-validation/range-image-vba-vs-officejs-getimage/images/
  vba-copypicture/
  officejs-getimage-batch8/
```

## Run

From this experiment directory in WSL, run both producers:

```bash
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File ./run-experiment.ps1
```

Run only the Office.js `Range.getImage()` side (no VBA capture first):

```bash
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File ./run-officejs-only.ps1
```

Useful variants:

```bash
# More representative sheets per workbook.
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass \
  -File ./run-experiment.ps1 -SheetsPerWorkbook 5

# Every visible worksheet.
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass \
  -File ./run-experiment.ps1 -AllVisibleSheets

# Try a different Office.js batching factor.
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass \
  -File ./run-experiment.ps1 -OfficeJsBatchSize 16

# Tune the first-time sideload activation loop.
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass \
  -File ./run-officejs-only.ps1 \
  -OfficeJsFirstActivationAttempts 5 \
  -OfficeJsActivationRetryDelaySeconds 20 \
  -OfficeJsRegistrationSettleSeconds 20
```

The Office.js side reuses the repaired add-in infrastructure from the drawing-bounds
experiment. Temporary workbooks are tagged with the documented auto-open Open XML
parts and launched through the Windows shell, matching Microsoft's sideload tooling.
The developer manifest is registered from a stable local-Windows path and remains
registered by default across runs.

Office's security model can still require one explicit trust/consent action the first
time the sideloaded add-in is activated. That is a one-time activation boundary, not
a required click for every workbook.

To make development sideload activation deterministic, the first workbook now gets up
to five activation attempts by default. The runner waits 10 seconds after registering
the manifest, then reasserts that registration before every attempt, creates a fresh
temporary workbook path, and launches a dedicated Excel process with `excel.exe /x`.
Microsoft documents `/x` as starting Excel in a new process. Failed attempts are
closed before the runner waits 10 seconds and tries again. This avoids reusing an
Excel process that cached the previous/stale developer add-in state.

Each first-workbook attempt has a 20-second result window by default; later workbooks
use the normal 15-second timeout and should not need the activation retry sequence.
After each successful result the task pane closes its own benchmark workbook and the
runner proceeds automatically.

Use `-TemporaryDeveloperRegistration` only when you deliberately want the developer
registration restored after the run; doing so can make later runs behave like a
fresh sideload again.

## Interpretation

Do not choose Office.js because a few individual ranges are faster. Compare the
aggregate time required to make the same PNG set available locally and the amount of
operational complexity needed to obtain it reliably.

A small or inconsistent speed advantage is unlikely to justify the Office.js
sideload/WebView/task-pane infrastructure when the VBA path is straightforward to
batch and automate. A large, repeatable advantage from `Range.getImage()` would be a
reason to reconsider that trade-off.
