# DeltaSieve configuration and reports

The standalone `deltasieve` application provides a web configurator and portable result exports. Its initial integration is under verification. The existing Enron study continues through its pinned runner and saved launch receipt.

## Build and configure

Use explicit working directories on a drive with sufficient free capacity. This PowerShell example places generated files on V:.

```powershell
$env:DELTASIEVE_DATA_DIR = 'V:\deltasieve-data'
$env:TEMP = 'V:\deltasieve-data\temp'
$env:TMP = $env:TEMP
$env:CARGO_TARGET_DIR = 'V:\deltasieve-build'
New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
cargo build -p excel-shrink-cli --bin deltasieve
& "$env:CARGO_TARGET_DIR\debug\deltasieve.exe" observatory --port 18778
```

The configurator opens at `http://deltasieve.localhost:18778/`. Loopback IP and `localhost` addresses on the same port are also accepted. The process binds to IPv4 loopback and checks the Host, Origin, and session token for local control operations. An occupied explicit port returns a bind error. Omitting `--port` selects an available port and prints its address. Use `--no-open` for an HTTP-only smoke test.

Reports and the download cache default beneath `DELTASIEVE_DATA_DIR/DeltaSieve`. Temporary working copies use the process temporary directory, so set TEMP and TMP as well. Explicit configuration paths take precedence. The configuration UI starts processing through its Start action.

## Command entry points

The VS Code task `DeltaSieve: Configure validation` invokes
`tools/start-deltasieve.ps1`. Its default working root is
`V:\deltasieve-data`. Pass `-DataRoot` or set `DELTASIEVE_DATA_DIR` to use
another drive. The starter places builds, temporary files, and logs beneath
that root. `-BinaryPath` uses an existing build, and `-Config` loads a saved
standalone configuration. An isolated HTTP smoke test verified the starter's
session and working paths on V:.

Choose `Resume a corpus study` in the web configurator and select the study's
`launch.json`. The adapter verifies its executable checksum, absolute paths,
request/output arguments and output lock before invoking the recorded runner.
Its own study lock remains authoritative if another launcher races the request.
New launcher logs are written beneath the existing output root in `web-launches`.
The returned link opens that study's Observatory after startup.

This path preserves the recorded binary and options. Changing study settings or
upgrading its producer still uses the existing corpus launcher while those
configuration contracts are being integrated. Standalone reports and corpus
studies retain their respective on-disk formats.

`deltasieve template` prints a configuration template. `deltasieve run --config PATH` executes a saved configuration. `compare --a SOURCE --b SOURCE --report DIRECTORY` compares existing sources. `open DIRECTORY` opens a retained standalone report. `export --help` describes the portable export command.

The cleanup executable exposes `excel-shrink-cli --describe-cli` for typed option fields. Its JSON describes the same Clap options used by the cleanup command.

`Add SheetShrink` uses the running DeltaSieve executable and its `shrink` entry
point, which dispatches to the shared Rust cleanup implementation. Its program
path is prefilled. `deltasieve shrink INPUT OUTPUT [cleanup options]` invokes
that entry point directly. The separate cleanup executable remains available.

A one-workbook test submitted the session's unchanged SheetShrink preset through
`/api/run`. Processing and native visual comparison completed with one processed
pair and zero failures. The portable report retained no equal PNG images.
Receipts are under `validation-temp/deltasieve-web-preset-smoke-20261108`.

`capture.retain_equal_images` controls equal-image storage and defaults to false.
The configuration form exposes the same setting. Completed comparisons retain
hashes and geometry receipts for discarded equal images. Images required by
differences remain available in the portable report.

## Verification status

The integration passed compilation of all targets, five focused watchdog tests, and 54 tests selected by the validation filter. Two fixture or explicit local tests were ignored. Three frontend tests cover relative evidence paths, shared comparison projection, and typed processor fields.

HTTP smoke tests returned the configuration page and idle session state. The updated server accepted the `deltasieve.localhost` Host and Origin headers, rejected a foreign Host with 403, and returned report and cache paths beneath the selected V: data directory.

A native CLI smoke test processed one known Original/Default workbook pair through the standalone coordinator. It published `completed_with_limitations`, zero different regions, no failed pairs, and no unmatched candidates. Its retained report also exported successfully to a separate static directory.

Three focused study-resume tests verify relative paths, missing/duplicate/mismatched
output arguments, a held output lock and a changed executable checksum. An HTTP
test using the active Enron launch receipt returned 400 for its held output lock;
the resume UI returned 200.

An isolated one-workbook corpus study completed native validation, exited, and
was then resumed through `/api/study/resume`. The resumed runner exited with no
reported error. Its corpus summary retained one completed comparison, zero
remaining workbooks and zero skips. All capture-attempt directories were unchanged,
confirming that the existing comparison was reused. Receipts are retained under
`validation-temp/study-resume-smoke-20261108`. Enron continued throughout the test.
The in-app browser resolved `deltasieve.localhost:18778`, displayed the initial
configuration dialog and switched to the study-resume form. Submitting the active
Enron receipt displayed the expected held-lock error. This check exposed missing
`updateMany` and `reset` methods in the shared comparison panel. Both methods are
implemented, and a focused frontend test covers the initial empty update and
report reset. Reloading the page cleared the observed startup error. Broader
browser checks for processing and portable-report exploration remain pending.
