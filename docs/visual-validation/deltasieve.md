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

The configuration service also accepts `configure.deltasieve.localhost` on its
actual port. Updated corpus-study hosts accept `observatory.deltasieve.localhost`
and print that address at startup. Requests remain confined to the exact Host,
Origin and port. Arbitrary `.localhost` subdomains are excluded. The add-in keeps
its existing HTTPS endpoint and certificate contract. Earlier pinned study hosts
continue to use their recorded loopback address until their host is updated.

Reports and the download cache default beneath `DELTASIEVE_DATA_DIR/DeltaSieve`. Temporary working copies use the process temporary directory, so set TEMP and TMP as well. Explicit configuration paths take precedence. The configuration UI starts processing through its Start action.

## Command entry points

### Validation partitions

The web configurator saves an optional `partition` object with `count` and
one-based `index`. For example, `{"count":4,"index":2}` selects partition two.
Names are assigned by the first eight SHA-256 bytes interpreted as a big-endian
unsigned integer, modulo the count. Adding other workbook names leaves the
assignment of existing names unchanged. Keep the same count when resuming a
partitioned experiment.

Selection happens after complete filename matching, preserving ambiguity
checks. The saved `selection` records available, selected and excluded pairs,
including empty partitions. Processing stages cover the complete source.
Persistent stage reuse can avoid repeating that preparation. The static report
includes the selection counts. A partition's result covers its selected pairs.

The current pinned Enron study continues with its original launch contract.
This standalone configuration does not alter its partition or admission settings.

The VS Code task `DeltaSieve: Configure validation` invokes
`tools/start-deltasieve.ps1`. Its default working root is
`V:\deltasieve-data`. Pass `-DataRoot` or set `DELTASIEVE_DATA_DIR` to use
another drive. The starter places builds, temporary files, and logs beneath
that root. `-BinaryPath` uses an existing build, and `-Config` loads a saved
standalone configuration. An isolated HTTP smoke test verified the starter's
session and working paths on V:.

The existing Windows VS Code task `Excel Shrink: Configure corpus run` now opens
the same web configurator. Its label remains as a compatibility alias for saved
task selections. Explicit PowerShell corpus configurations and the WSL launcher
remain available through their existing scripts while profile migration proceeds.

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

### Publication review of static exports

Export also writes `publication-review.json` and displays a review notice in the
exported overview. The inventory scans copied run, comparison and reuse-identity
JSON for local-path syntax, identity/credential fields, and URL credentials.
Entries identify the relative file, JSON pointer and concern category without
copying the field value. It counts every match while retaining at most 1,000
locations, with an explicit truncation flag.

This is a bounded review aid. It preserves scientific reports and their integrity
manifests. Workbook names, formulas, screenshots and corpus distribution rights
need a separate publication decision. A zero metadata count leaves that review
requirement in place. Exported evidence may contain confidential workbook content.
Keep a working export private until its publication review is complete.

The portable comparison page renders retained formula-dependency paths using the
same presentation module as the live Atelier. Each explanation is matched to its
exact comparison pair and region. It shows recorded values, formulas, arrows and
the supported volatile-call highlight without requesting an API. Missing or
budget-omitted explanations remain explicit. Older reports do not acquire new
dependency evidence merely by being exported.

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

The ordinary `deltasieve run --config` entry point was exercised again on
2026-11-08 using the frozen Enron sample member `0966d3e126908018b94a`. Both
sources were copied to independent folders with matching workbook names.
The coordinator completed one pair with five selected regions and five captures
per rebuilt state. It reported `completed_with_limitations` and no differences.
Its `Capital Structure` finding retained `blank_margin_ignored=true`, matching
the independent reference's verified one-pixel white exterior observation.
Verified-equal images were discarded with their integrity receipts retained.
The run used the actual `deltasieve.exe` entry point and resident services,
restored Enron admission to four groups, and retained its working report under
`G:\deltasieve-work\deltasieve-native-entry-20261108`.

The app-name integration test initially exposed missing `metadata-store`
dispatch in `deltasieve.exe`; `95c2006` centralizes shared native service dispatch.
Both app names now pass the same internal-service test. Study receipt validation
also accepts the explicit `observatory.deltasieve.localhost` alias while rejecting
foreign suffixes, unlisted names, missing ports and unsupported schemes.
