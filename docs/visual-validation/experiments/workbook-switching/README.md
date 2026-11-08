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
