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

### Preparation Observatory

`xlsx-convert.exe --web` and `excel-check-performance.exe --web` serve their
configuration and progress UI on loopback. Starting either executable without
input/output arguments also enters web mode. `--port 0` selects an available
port and prints its URL. Input/output/worker options prefill the form. Processing
starts when the user presses Start. Both applications embed the same frontend
and call the native library in-process through `preparation-web`.

The interface adapts the supplied SheetShrink Observatory's renderer, worker
lanes, CSS and distribution scenes. The source archive SHA-256 is
`a3d99a344226fcf2268950451148e86e67df3b8e8590a57a72b07826ed285e31`.
The synthetic producer and predetermined job schedule have been replaced by an
incremental native-event projection. Conversion shows workbook progress,
throughput, ETA and failures. Check Performance additionally shows workbook and
worksheet size distributions using before/after optimization controls.
Each worker lane contains workbook jobs on a single level.

Start, stop and live worker changes use token-protected JSON endpoints. Host
and Origin checks restrict requests to the printed loopback authority. State
polls carry a cursor and send subsequent events plus aggregate counts. Initial
connection, run changes and expired cursors receive a bounded snapshot.
The projection retains 256 recent jobs and 512 events. Global counts survive
history eviction. Full receipts and an append-only `<output>.events.jsonl`
remain on disk. A normal web run creates this event file automatically.

Stop requests terminate the owned Excel pool, retain completed outputs and
leave unstarted inputs queued. They may interrupt the active workbook.
The source manifest is published from successful conversion receipts, including
when a conversion run is stopped. Check Performance continuation still uses
verified `--reuse-from` artifacts. The web host's live projection belongs to its
process lifetime. Reopening an old result directory as a static report is a
separate workflow.

Frontend sources are under `crates/preparation-web/ui`. Run `pnpm install`,
`pnpm check`, `pnpm test` and `pnpm build` there when changing them. The lockfile
pins dependencies. Generated `assets/app.js` and `assets/app.css` are committed
and embedded in the binaries, so deployed applications need no Node runtime.
Bundled dependency licenses are retained under `ui/licenses`.

### Native UI acceptance

The small native test used a genuine BIFF8 XLS file containing A1 = 7 and B1 =
`A1*2`, plus an XLSX with 999 formatted blank cells. Both were selected through
the conversion web form with two workers and an 8192 MiB physical/commit
reserve. Both completed in approximately 2.99 seconds. Their generated source
manifest then drove Check Performance through its web form in approximately
6.50 seconds. Outcomes were `no_optimization_needed` and
`optimization_confirmed`. The measured optimization controls totaled 23,103
bytes before and 17,302 after. XML inspection confirmed A1 = 7, B1 = 14 and the
formula `A1*2` in all converted/before/after files. These tiny-file timings
describe functional acceptance only.

A stop test retained seven completed outputs, recorded one interrupted active
input and left 192 of 200 duplicate input occurrences queued. All owned Excel
processes exited. A separate live worker-change test confirmed two ready, busy
native sessions after changing the target from one to two. Both dashboards
were visually inspected in the browser. This testing also exposed and fixed
extended-length TEMP paths rejected by the .NET Framework helper compiler.

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
