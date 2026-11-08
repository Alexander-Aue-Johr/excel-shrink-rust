# Workbook switching in one owned Excel process

The native probe `native_xlsx_pair_opens_in_the_same_owned_excel_process` isolates
COM workbook opening from Excel process startup and Office.js initialization.
It uses the existing conversion worker's process ownership and private desktop.
COM is obtained from the exact owned Excel window, with PID verification, rather
than from a global active-object lookup.

The worker retains its seed workbook and opens Original and SheetShrink copies
twice each. Every sample must use the same Excel PID. The probe verifies both
source hashes after the requests and confirms that the owned Excel identity has
exited after pool shutdown. Saved outputs and receipts remain in its unique
`native-workbook-switch-*` temporary directory for inspection.

## Running the bounded probe

Select two small prepared XLSX files and set the following variables before
running the ignored test serially. Use a temporary directory with sufficient
space outside the system drive.

```powershell
$env:TEMP = 'V:\validation-temp'
$env:TMP = $env:TEMP
$env:NATIVE_REUSE_ORIGINAL = 'V:\fixtures\original.xlsx'
$env:NATIVE_REUSE_SHRUNK = 'V:\fixtures\sheetshrink.xlsx'
cargo test -p excel-shrink-cli --lib native_xlsx_pair_opens_in_the_same_owned_excel_process -- --ignored --nocapture --test-threads=1
```

The test opens inputs read-only with `UpdateLinks = 0`, macros and events disabled,
and manual calculation. It saves separate XLSX outputs using the existing
conversion path. `worker_startup_seconds` includes worker staging and readiness.
Each sample's open time measures the COM open call in that retained process.

This probe does not load the capture add-in, perform rendered comparison, or
establish the cost or correctness of Office.js context switching between files.
It is a measurement prerequisite for a future capture-worker implementation.

## Measured pair on 2026-11-08

The tiny prepared Enron pair used in the startup investigation was processed in
one native run. Worker staging through readiness took 5.943 seconds once.

| Request | Input | COM open, seconds | Save copy, seconds | Close, seconds |
| --- | --- | ---: | ---: | ---: |
| 1 | Original | 0.677 | 0.744 | 0.870 |
| 2 | SheetShrink | 0.256 | 0.734 | 0.639 |
| 3 | Original | 0.320 | 0.652 | 0.698 |
| 4 | SheetShrink | 0.296 | 0.900 | 0.787 |

All four receipts retained the same Excel PID and worker session ID. Source
hashes were unchanged and final owned Excel exit was confirmed. The native test
completed in 15.43 seconds, excluding Rust compilation. Its local receipt is
`native-workbook-switch-c4c0c28b-8189-4478-9218-5fa9cfaed4a7/test-results.json`
under the configured temporary directory. This is one small, serial trial.
The earlier launch-to-add-in-ready numbers have a broader scope and must not be
presented as a directly matched speedup over these COM-only open measurements.

## Office.js readiness after switching

`office-ready.ps1` is a separate exploratory harness using the existing native
ownership helpers. It retains one seed workbook and Excel process, one manifest
ID, one isolated browser profile and one resident HTTP host. Each test workbook
is copied, receives the normal auto-open metadata, and is opened read-only through
the owned COM application. The host verifies its new workbook name.

The harness dispatches no comparison commands. It uses the existing authenticated
startup reset between files, which is explicitly rejected after data work begins.
This does not implement the generation boundary needed between real comparisons.

The successful trial `office-ready-switch-20261108-06` produced four distinct
Office.js document IDs in the same window-verified Excel PID. The process was
confirmed ended and input hashes were unchanged.

| Request | Input | COM open, seconds | Open through verified Office.js ready, seconds |
| --- | --- | ---: | ---: |
| 1 | Original | 1.693 | 2.634 |
| 2 | SheetShrink | 0.386 | 1.109 |
| 3 | Original | 0.280 | 1.114 |
| 4 | SheetShrink | 0.339 | 0.777 |

Worker readiness before these requests took 4.928 seconds. Trial 04 instead
created a new manifest ID and HTTP host per file. Its first workbook became ready
in 2.239 seconds, but the second timed out waiting for Office.js. This suggests
retaining the registration/host is worth pursuing; it does not isolate which
change resolved the failure. Earlier setup trials failed on unavailable hashing
cmdlets, a missing workbook helper, an incorrect zero-based worksheet index and
an omitted launcher token. They provide no successful switching measurements.

The final harness also checks that only the seed remains after each close and
that all four document IDs differ. Trial 07 passed these added assertions and
again confirmed final Excel exit and unchanged source hashes.

## Pixel captures after switching

Optional `-CaptureRange 'A1:C10'` replaces the taskpane page with a small probe.
It synchronizes the workbook name and `Range.getImage`, decodes the PNG to RGBA
in a canvas, hashes the pixel bytes with Web Crypto and posts dimensions and
the digest with its readiness observation. It checks repeated captures of each
source, source file hashes, distinct document identities, workbook count and
the owned Excel PID. `-ExpectDifferentPixels` additionally requires different
digests for the two supplied sources. This option requires `CaptureRange`.

Trial `office-pixels-switch-20261108-01` used the original and SheetShrink pair.
All four captures were 147 by 181 pixels with the same RGBA digest. Excel
initialization took 6.227 seconds. Opening through completed capture and ready
took 2.239, 1.411, 1.002 and 0.832 seconds, in one Excel PID.

Trial `office-pixels-switch-20261108-02` used the original and a separate control
copy with B4 replaced by inline text `CHANGED`. The A/B/A/B sequence produced
two distinct digests, with each source's digest and dimensions repeating exactly.
The changed control was 147 by 183 pixels, the original 147 by 181. Initialization
took 4.418 seconds. Open, capture and ready took 1.578, 0.749, 0.848 and 1.515
seconds. Both trials confirmed final owned Excel exit and unchanged input hashes.

These small captures establish that fresh Office.js document contexts capture
the corresponding workbook after a warm COM switch. They dispatch no resident
comparison commands and do not exercise command completion, retained-image
release, production generation boundaries, rebuilds or multiple workers. Those
remain required before using warm switching in the corpus pipeline.

## Resident commands across isolated host sessions

Optional `-ResidentCommands` uses the production taskpane and resident command
handlers. It captures a deliberately small Sheet1 extent, uploads its PNG through
the normal evidence endpoint and issues an unconditional `release`. The fixture
extent is zero-based bottom row 10 and right column 2, producing A1:C11. This
mode requires two deliberately different fixtures and checks repeated tile
metadata and SHA-256 values across A/B/A/B. It cannot be combined with the
minimal `CaptureRange` page.

After release, the workbook is closed and only the seed remains. Before opening
the next workbook, the harness confirms the old owned host process has exited
and starts a new host on the same origin with a separate session directory.
The manifest identity and Excel process stay fixed. This retains the existing
host protocol boundary and its immutable evidence. It avoids relaxing the
startup reset guard after data work has begun.

Trial `office-resident-switch-20261108-01` failed on missing explicit overflow
and drawing flags in the test command. It provides no successful capture result.
Trial 02 completed four capture, upload and release sequences in Excel PID
38324. Trial 03 added old-token rejection checks and completed in PID 1968.

Trial 03 initialized Excel in 4.205 seconds. The four workbook-open through
ready observations were 1.895, 2.000, 1.233 and 1.127 seconds. These observations
exclude the subsequent resident capture/upload/release commands. All repeated
captures matched. The distinct control rendered different pixels. The four
uploaded PNGs were independently hashed from disk and matched their receipts,
with 6,029 bytes for A and 6,321 for B. Every release reported zero retained
capture plans, image bytes and geometry-cache entries. All nine attempts to use
the previous host's client token for ready, result or image upload returned
HTTP 400. Final Excel exit and unchanged source hashes were confirmed.

This experiment supports retaining Excel while replacing its per-comparison
HTTP host at a stable origin. Production integration still needs pool admission,
source and comparison identity binding, interruption recovery, matched rebuild
and pixel checks, and concurrent worker isolation. The single-worker probe does
not establish those properties or change the corpus runner's lifecycle.
