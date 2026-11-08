# Implementation baseline for the approved delivery plan

## Current verified checkpoint

The initial observations below are historical checkpoints. After the selective
font-sentinel refresh, production uses immutable runner commit `317200b`, binary
SHA-256 `cb881f88de082046a315476d98b743d4acecd7bb2f438b3911af6b08a0bd10e6`,
and PID 50844. Its saved `launch.json` contains the executable, checksum, complete
arguments, environment and output root. At the later check, 5,175 visual
comparisons were complete, five were skipped, four were active and 11,619 remained.
Preparation still accounts for all 20,872 inputs and 20,503 prepared packages.

The independently deployed Observatory host uses commit `5df6cbc` and PID 37524.
Its deployment receipt is under
`G:\deltasieve-work\host-reconcile-20261108`. The current address is
`http://observatory.deltasieve.localhost:18777/`. The host's source assets are
served from this checkout. Changing a browser module does not replace the pinned
scientific runner. Static exporter and manuscript changes likewise do not upgrade
that runner.

Both repositories were fetched again after the integration work. The reusable
remote branches remain `origin/codex/sheetscope-validation` at `a7445eb` and the
paper repository's `origin/codex/corpus-preparation-snapshot` at `98c4e9d`.
The current software and paper branches are `codex/datagov-validation-watch`
and `icde2026`. Unrelated paper-repository changes remain outside our commits.

### Recover this exact study

1. Read the study's `launch.json` and verify its recorded executable checksum.
   Inspect the running PID and its executable before deciding it has stopped.
   The authoritative `.run.lock` must be available. Never launch a second writer
   against an active study.
2. The web configurator's Resume existing study action accepts this launch
   receipt. It validates the recorded paths, checksum and lock before spawning
   the recorded argument array and environment. The runner reacquires its lock.
   The ordinary completed-study resume smoke test preserved its capture-attempt
   directories and reused the existing result.
3. The independently upgraded host is newer than this pinned runner. If it
   remains after the runner stops, an exact-build resume will reject that host
   explicitly. Confirm its port, study root, executable, PID and creation time,
   then stop only that owned host before resuming. The pinned runner can then
   start its matching host on the same port. Preserve `control-room/control.json`.
4. Check `/health` and `/api/summary`, confirm `runner_alive`, the intended study
   root, four-group target, retained preparation counts and new comparison
   progress. A ready webpage alone does not establish that scheduling resumed.

The 1,234 refreshed preparation records and ZIPs have a separate hash-verified
backup under `G:\deltasieve-work\enron-font-fix-refresh-20261108`. Independent
verification found all other 19,269 preparation ZIPs unchanged. Ordinary recovery
does not use `--refresh-preparation` or a subset workbook list.

Open investigations retain their evidence. Group `cade66d204a8e1b3760d` needs a
bounded-drawing planning resolution and retry. Group `08d63692cee7c0416461` timed
out during rebuild and contains XLL function calls without external-link parts.
Repair-required inputs retain their explicit skip reasons. The difference ledger
continues separately from these baseline and recovery checks.

The active Enron runner was verified through process PID 17640, its launch receipt, and the live `/api/summary` endpoint on port 18777. Its executable is pinned to commit `abc9e79` and binary SHA-256 `11158836206f7bb9d0898d7851e79190c50f9ef58d67b8d974505bf7d93e09f3`.

The run root is `V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\enron-20261107`. Its launch receipt retains the complete argument list and request path. Recovery must use that receipt, the same output root, and the recorded compatibility settings. Verify the process identity and terminal state before recovery.

## Corpus and execution

The selected archive contains 20,872 inputs. Preparation is terminal for all inputs, with 20,503 prepared. The visual scope is 16,799 workbooks. At this observation, 4,080 were compared, three were skipped, four were active, and 12,716 remained. The rolling throughput was 324 completed workbooks per hour. These figures are a baseline snapshot.

The active configuration uses Default, four comparison groups, resident capture, deterministic chart selection, hidden Excel, disabled Check Performance, and disabled equal-image retention. The primary execution strategy is paired in-place rebuild. Live reports identify capture policy `targeted-content-grid-v9`.

The current corpus guide contains historical v3/v8 policy descriptions and a multi-instance description that differs from this active strategy. Reconcile those passages during the contract review using the current implementation and retained evidence.

## Standalone branch integration

The local remote-tracking branch `origin/codex/sheetscope-validation` provides a web configurator, standalone coordinator, source acquisition, result reuse, and portable-report components. Its library target shares `src/main.rs` with thin binary adapters. It currently calls the tool SheetScope and exposes a `sheet-scope` executable.

The branch comparison spans 176 files. Its tree predates current SQLite documentation, capture experiments, process metric presentation, inspection windows, and other changes on the active branch. Integrate the relevant standalone components with an explicit compatibility review of those current facilities.

The standalone workflow supports external processing commands, which require review against the requested native SheetShrink integration. Its bounded monitoring and static report components are candidates for reuse.

The standalone capture adapter passes `-SheetScope` to `run-targeted-comparison.ps1`, while the current script dispatches `regression targeted-compare` and has no such parameter. Adapt that boundary explicitly. Its validator source fingerprint also lists comparison files directly, so reconcile the source inventory with current modules and runtime assets. The current launcher accepts deterministic chart selection and distinguishes validated geometry progress from heartbeat activity.

The shared application entry point was extracted and compiled with all targets in `9dd1f1f`. Machine-readable cleanup metadata is available through `--describe-cli` in `d3329dd`. The focused metadata test and normal CLI execution passed, exposing 35 named options. These changes provide initial integration prerequisites.

Fresh paper projects were created under `papers/sheetshrink` and `papers/deltasieve` in the paper repository and pushed on its `icde2026` branch as `d29ea94`. Both initial compilation scaffolds produced PDFs using TeX Live with build output on V:. Their full research-based manuscripts are pending.

An isolated Enron opening failure in group `ee6c5aa544fd08d1d31d` was retried through the regular restart API. The initial attempt timed out after 180 seconds while the Default add-in had no readiness confirmation. Retry attempt `6e380811-7883-4ae9-9e93-fd900ade0a86` completed with status `unchanged`. The initial failure's cause remains unresolved.

## Performance handoff verification

The review archive on R: contains four isolated proposals against `a7445eb`.
Proposal D rejects the dominant style after its unchanged selection when it
neither applies a font nor has an existing font-only representative. Both dense
promotion branches require that style's `apply_font`; the sparse branch requires
`apply_font` on one of those two targets. The early rejection therefore preserves
the target selection. It avoids later default-style lookup and target traversal.
It does not avoid the preceding cell scan or frequency count.

The implementation passed six focused promotion tests, including a dominant style
without `apply_font` that still has a valid separate font representative. The
complete Core library suite passed 277 tests with one ignored benchmark.

A release build containing D was compared against the pinned `abc9e79` binary on
the report's SHA-256-verified originals for Förderprogramme, Kernhaushalt and
Verkehrsunfälle. All 1,213 decompressed ZIP members were identical, with no added
or missing members. Duplicate member names would fail this check. This establishes
output identity for these three inputs. The probe receipts are retained beneath
`validation-temp/dominant-font-guard-20261108`, including input paths, binaries,
individual timings and `member-comparison.json`.

The single observations ran while Enron and, during the baseline, compilation
were active. They cannot establish speedup. Controlled repeated timing remains
open before deployment into the pinned Enron runner. Proposals A, B and C remain
unimplemented.

## Capacity and continuity

Initial free capacity was approximately 85 GiB on C:, 317 GiB on V:, and 1,374 GiB on G:. Prefer V: for working data and evaluate G: for overflow if needed. Preserve the system-drive reserve and record each owned working directory.

The pipeline continues during development. Native experiments use the approved bounded test windows and confirm resumed Enron progress after any interruption. Outstanding visual findings remain subject to their retained evidence and individual review.
