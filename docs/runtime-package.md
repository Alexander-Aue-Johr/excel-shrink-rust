# Windows runtime package

## Shared native automation and standalone applications

`corpus-runner` owns source occurrence identities, atomic artifact writes,
Windows process ownership, physical-memory and commit checks, ZIP directory
measurements and optional ordered events. `excel-automation` owns the native
Excel pool and conversion/Check Performance orchestration. Both new applications
call this library directly. They have no dependency on the DeltaSieve CLI.
The historical CLI entry points delegate to these same implementations.

```powershell
cargo build -p xlsx-convert -p excel-check-performance --release -j 2
xlsx-convert.exe --input 'V:\inputs' --recursive --output 'V:\converted' --workers 4
excel-check-performance.exe --source-manifest 'V:\converted\sources.json' --output 'V:\optimized' --workers 4
```

The runtime packager includes these two executables when present in its binary
directory and verifies them through the same per-file integrity manifest.

Output directories must be new. Conversion retains frozen source copies,
converted XLSX files and receipts, and publishes a source manifest accepted by
Check Performance. Check Performance also accepts `--input` XLSX paths or
directories, with optional `--recursive`, and retains its existing verified
`--reuse-from` continuation mechanism. Conversion currently requires a new
output directory and has no continuation option.

Both apps accept `--reserve-mib` and `--events`. The reserve applies to the lower
of available physical RAM and system-wide commit headroom, with a 512 MiB floor.
The default retains one eighth of installed RAM, bounded at 8 GiB. Reaching that
reserve stops the owned pool and preserves unstarted jobs as queued. Admission
keeps a further 512 MiB startup margin. These sampled guards cannot prevent an
unrelated process or one Excel request from allocating memory between checks.

The optional event file must be new and its parent directory must exist.
Each JSON line carries a version, ordered sequence, elapsed milliseconds,
operation, event type, workbook identity, dispatch slot and payload. Events
describe run start/end, reuse and job start/completion/failure. Dispatch slots
identify orchestration lanes, not native process IDs. GUI consumers can pass an
`Observer` through `RunContext` to the same backend. Observer failures are
retained and fail the run at its boundary after preserving native results.
Consumers must deliver events promptly and offload rendering or network work.

ZIP size measurements use the central directory without inflating worksheet
XML. Receipts distinguish complete file bytes, compressed member bytes and
uncompressed XML bytes. Worksheet identifiers here are ZIP part paths. Resolving
display names additionally requires workbook and relationship metadata.
Check Performance retains separate measurements for baseline, saved original,
before optimization and after optimization, so ordinary Excel save effects can
be distinguished from optimization effects.

The native helper assets still partly reside in the historical CLI source tree
and are embedded into `excel-automation` at build time. This is a transitional
source-layout dependency, not a runtime dependency on the CLI executable.
Conversion reuses Excel sessions. Check Performance retains fresh Excel per
input while continuously refilling worker vacancies.

### Observatory demo assessment

The supplied SheetShrink Observatory demo has a synthetic producer, replay
clock, worker lanes and before/after distributions. It is suitable as a visual
starting point. Its producer precomputes future jobs and end times. A production
adapter must instead consume incremental events and retain explicit failed,
skipped and interrupted states, with bounded visible history for large corpora.
The standalone apps expose this event seam. The demo is not yet connected.

For conversion, show workbook progress, throughput and ETA. For Check
Performance, show workbook lanes and before/after sizes. Sheet/chunk lanes and
cleanup distributions apply only when the selected operation provides those
measurements. The static UI can be served by a Rust host without a Node runtime.
Replay should consume the same events as the live projection. Durable receipts
remain authoritative when optional UI telemetry is incomplete.

Build the Windows entry points with a configured build directory on a working
drive, then package their outputs into a new directory. Python 3.9 or newer and
Git are required by the packaging tool. The resulting runtime requires Windows
and installed Excel for native comparisons.

```powershell
cargo build -p excel-shrink-cli -p sheetshrink --bins --release -j 2
python tools/package-validation.py `
  --binary-directory "$env:CARGO_TARGET_DIR\release" `
  --output 'V:\releases\deltasieve-candidate'
python tools/package-validation.py --verify 'V:\releases\deltasieve-candidate'
```

The parent output directory must already exist. The tool rejects an existing
destination. An interrupted copy retains an `.incomplete` marker and cannot pass
verification. Reattempt publication into a new directory after inspection.

The package contains supplied `deltasieve.exe` and `excel-shrink-cli.exe`, tracked
runtime Office/web assets and their PowerShell/C# process-management helpers.
It excludes test directories, local corpora, generated dependencies and ordinary
source files. `release-manifest.json` records every packaged member's byte size
and SHA-256. Verification checks required members, hashes, unexpected files and
path confinement after relocation.

When `sheetshrink.exe` is present in the supplied binary directory, it is included
and verified under the same manifest. Historical two-binary packages remain
verifiable. Build both packages with the command above for a two-application
release.

The asset checkout commit and dirty state are recorded separately from supplied
binary hashes. The packager does not infer which source revision produced an
executable. Preserve the actual build receipt when freezing a scientific
release. A verified package proves byte integrity against its local manifest,
with no claim of independent signature-based authenticity.

Run `deltasieve.exe` to open the standalone configurator. Keep the complete
directory layout when moving it. Use a working drive for TEMP, TMP,
EXCEL_SHRINK_LOG_DIR and EXCEL_SHRINK_VISUAL_LOG_DIR. Supplied configurations
should put corpus inputs, prepared states and outputs on working drives too.
The Office asset resolver supports an explicit EXCEL_SHRINK_CAPTURE_DIR for
controlled deployments. An invalid explicit path fails.

This is a packaging prerequisite, not the final distributed evaluation release.
The full roadmap still requires independent application defaults, replay,
export/merge and ordinary study start/resume acceptance after relocation.

## Relocation acceptance on 2026-11-08

Four package tests passed, covering member selection, relocation, corruption,
existing destinations, unexpected files and interrupted publication. A real
debug runtime package contained 98 files and passed verification before and
after moving from `package-01` to `relocated-01` under
`G:\deltasieve-work\runtime-relocation-20261108`.

The packaged Observatory started from an empty working directory without an
explicit capture-directory override. A unique temporary probe served from its
packaged web tree established the selected asset location. The test host was
closed, the probe removed, and all 98 package files verified again.

The relocated executable then completed one existing Original/Check Performance
fixture pair through the ordinary `compare` command from the same empty working
directory, in approximately 59.04 seconds. It reported one processed pair, zero
failed pairs, and `completed_with_limitations`. Five Original-rebuilt and five
candidate-rebuilt region captures were retained as observations. The report
recorded one differing region, with geometry and images preserved. This smoke
test establishes execution and evidence publication, with no new attribution of
that difference or claim of visual equality.

Both Excel PIDs recorded by the comparison were absent after completion. Enron
admission was restored to four and a live query confirmed four active groups
and continuing completions. Private acceptance receipts are
`web-relocation-acceptance.json`, `native-relocation-process.json` and the
`native-report-01` directory beside the package. This covers one standalone
pair. Relocated archive-study start/resume and other release gates remain open.
