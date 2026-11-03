# Prepared Excel controls

`tools/prepare-excel-corpus.py` prepares Excel controls before a visual-validation run. Preparation results have status `artifacts_prepared`. Visual comparison remains pending.

The runner reads `config.json` and `cases.json` from its `--root` directory. It verifies the prepared original against its study record and generates Default with the pinned executable. Setting `default_source` to `prepared` reuses the bundled Default instead and records its producer checksum and historical shrink time. Each attempt uses a private desktop and an owned Excel process group. Preparation keeps one workbook open per worker. Two shared startup slots bound cold Excel/WebView launches while ready workers continue independently. Windows releases the startup mutexes if a worker exits unexpectedly.

## Artifacts

The original and Default inputs are copied before Excel opens them. The saved controls use ordinary Save. Exact byte equality permits omitting the independent Default lane under the validation specification. Its copied input is then labelled `default.identical-input.unopened.xlsx`, with the omission recorded in `default_state`. Each rebuild control starts from a filesystem copy of its saved parent, opens and saves that copy, then performs `fullRebuild` and Save. The intermediate saved copy is retained.

Check Performance starts from a copy of the saved original. Its baseline, Optimize All result and ordinary saved result are retained. A separate reopened copy produces the rebuilt control.

The Save As control runs Optimize All independently from the same saved original. Excel performs Save As, then rebuilds and saves the still-open workbook. Both outputs are retained. This preserves the distinction between a reopened rebuild and a same-session rebuild.

`preparation.json`, `operations.json` and the attempt result record the inputs, outputs, timings and failures. Each host operation samples the owned Excel process before and after execution, including private bytes, working set, virtual bytes, CPU and handles. Observed process peaks are retained. Native heap size is unavailable through these process counters and is recorded as null.

Each attempt has a separate temporary directory under the configured working root. After durable artifacts and failure diagnostics have been retained, cleanup removes both that attempt and its temporary files, including Office diagnostic logs. Shared temporary directories can accumulate substantial Office logs across otherwise completed attempts.

## Cache reuse

Set `EXCEL_SHRINK_CONTROL_ARTIFACT_CACHE_DIR` to share saved/rebuilt controls with targeted validation. Check Performance uses the existing performance cache. Keys bind the original checksum, saved-parent package fingerprint, Excel environment and producer recipe. Restored files are checksum-verified. Historical measurements remain identified as historical.

Rebuild reuse requires a dependency audit. Volatile or unknown dependencies require a fresh rebuild. The normal validation path still performs its opening and visual checks.

## Running and stopping

`tools/analyze-prepared-sizes.py` inventories existing Original/Default pairs through the ZIP directories. Stored outer members are accessed as bounded file views. Worksheet XML payloads are neither read nor extracted. Its CSV files rank complete XLSX sizes and sheet-part sizes separately. The threshold lists identify workbooks qualifying by either total XLSX reduction or at least one sheet's XML reduction. Sheet identifiers are package part paths. Reusing prepared Default keeps the subsequent Excel comparison aligned with this screening baseline.

Run `python tools/prepare-excel-corpus.py --root <run-directory>`. Keep the pinned executable and host sources unchanged during a run.

The pilot compares independent cold-cache runs over eight input-size strata. It selects the highest successful throughput among trials with at least 75% success. The estimate comes from this small stratified sample and can change as larger or unusual workbooks are processed.

The main batch reuses successful pilot outputs, processes smaller bundles first and retries failed cases once with a longer operation timeout. Admission checks available RAM and free disk space. A `stop.request` file in the run directory stops new admissions after active attempts finish. The configured deadline has the same effect. A whole-workbook timeout bounds active attempts beyond the deadline.

Completed results and durable artifacts survive a restart. Existing terminal results are skipped on restart, including failures. Review those records before scheduling another retry. Run one controller per output directory.
