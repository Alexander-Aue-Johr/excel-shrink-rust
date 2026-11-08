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

`deltasieve template` prints a configuration template. `deltasieve run --config PATH` executes a saved configuration. `compare --a SOURCE --b SOURCE --report DIRECTORY` compares existing sources. `open DIRECTORY` opens a retained standalone report. `export --help` describes the portable export command.

The cleanup executable exposes `excel-shrink-cli --describe-cli` for typed option fields. Its JSON describes the same Clap options used by the cleanup command.

## Verification status

The integration passed compilation of all targets, five focused watchdog tests, and 54 tests selected by the validation filter. Two fixture or explicit local tests were ignored. Three frontend tests cover relative evidence paths, shared comparison projection, and typed processor fields.

HTTP smoke tests returned the configuration page and idle session state. The updated server accepted the `deltasieve.localhost` Host and Origin headers, rejected a foreign Host with 403, and returned report and cache paths beneath the selected V: data directory. Browser hostname resolution, a real native comparison through the standalone coordinator, existing-study resume integration, and full browser interaction remain pending.
