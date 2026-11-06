# SheetScope: standalone validation behavior

This specification describes the standalone workflow introduced by the 2026-11
SCE patch. **SheetScope** is the working tool name; `sheet-scope` is its CLI.
The existing Excel Shrink cleanup and corpus-study commands remain available.
No repository split or relocation of the scientific comparison sources occurs.
See [the operating guide](../visual-validation/sheet-scope.md) for commands and
configuration examples.

## SV1. Responsibilities and entry points

The two binaries share the existing crate through a library target whose root
remains at `src/main.rs`. Thin binary adapters select the cleanup CLI or the
standalone validation CLI. This preserves source locations and avoids compiling
two independently edited copies of the validation implementation. It does not
yet produce a separately distributed validation crate.

| Responsibility | Source under `excel-shrink-cli/src/` |
| --- | --- |
| Configuration and CLI | `validation.rs` |
| Run state, pairing, persistence and capture adapter | `validation/{run,pairing,storage,capture}.rs` |
| Input acquisition and download cache | `validation_sources.rs` |
| External program invocation and process ownership | `validation_process.rs`, `validation_process/invocation.rs` |
| User-selected OOXML fingerprints | `validation_reuse.rs` |
| Evidence packaging and shared static presentation | `validation_report.rs`, `validation_report/{evidence,site}.rs` |
| Loopback configuration and monitoring UI | `validation_room.rs` |
| Scientific selection, native capture and comparison | Existing `regression/targeted_comparison*`, `capture_host*` and `office-capture/` |

Starting `sheet-scope` without arguments opens the Observatory with an incomplete
configuration. A run starts only through its Start button or an explicit CLI run
command. `compare` selects existing A/B sources. `run --config` accepts a JSON
configuration. `open` generates/opens the saved report viewer; the Observatory's
report mode also selects an existing run. `export` creates a new standalone site.

`observatory --a SOURCE --excel-shrink EXE` prefills a file-processing stage.
Optional `--report`, `--output`, `--max-processes` and `--rayon-threads` override
its defaults. Omitted workbook output is temporary. An omitted report destination
gets a unique local run path, which is created only when validation begins.
The default external process count is the available logical processor count;
the Shrink preset uses one Rayon thread, one active workbook and one writer per
file process. Increasing Rayon threads multiplies the potential CPU demand.

Configuration opens above the monitor and closes after a successful Start
request. The monitor reuses the Observatory KPI styles and targeted process,
heartbeat and capture-flow cards. Its workbook list is paginated; full historical
indexes are excluded from status polling. It does not enable the legacy corpus
runner's live control endpoints or its three-dimensional sheet studio.

Windows Browse actions open native folder/file dialogs. Sources provide separate
folder and file buttons, and the executable uses a file dialog. A new report or
output destination is generated beneath the selected parent. Other platforms
retain the server-side directory browser.

The Observatory listens on loopback, uses an automatically assigned port by
default, and requires same-origin requests plus its session token for actions.
Its explorer selects paths on the host computer, not browser-uploaded workbooks.
CLI/configuration and UI actions call the same coordinator. Config-file-relative
paths resolve against that file's directory; interactive relative paths resolve
against the process working directory. Arbitrary argument strings are not
implicitly rewritten into filesystem paths.

## SV2. Inputs, outputs and reports have separate lifetimes

Input A and optional input B independently accept local directories, local ZIPs,
HTTP(S) ZIP URLs and individual workbook files. Acquisition recognizes XLSX, XLS,
XLSM and XLSB; that does not enlarge the targeted renderer's XLSX-only contract.
Directories are read in place. ZIP members needed for processing are extracted
under owned working directories with relative names preserved. Invalid paths,
aliases, links and duplicate archive members are rejected before extraction.

Downloaded archives are cached by URL SHA-256 and stored as immutable versions
named by their content digest. Reuse verifies length and digest. Explicit refresh
downloads a new version without replacing bytes another run may be using.
Concurrent/stale cache locks produce an explicit error; the runner does not
guess that an existing lock is abandoned. ZIP is the only implemented corpus
archive format. Downloads use the system `curl` executable.

Processing output defaults to **temporary**. An optional final directory or ZIP
is independent of `report_dir`; all must be disjoint from managed caches and
must not be created inside a source directory. Permanent destinations must be
new paths. The final processing stage writes directly into a requested output
directory. An output ZIP is published after processing and before validation,
so a later capture failure does not discard an explicitly requested archive.
Intermediate files needed to produce the ZIP remain temporary.

Each run requires a new report directory. Completed per-pair evidence is packaged
before its capture working directory is deleted. Original-resolution comparison
images, exact difference masks where available, report metadata and integrity
receipts survive temporary workbook deletion. The saved viewer does not need the
original workbooks or processor executables. Logs and configuration retain
provenance; they are not a scrubbed publication artifact.

Temporary work is removed after normal completion or confirmed failure cleanup.
The legacy renderer's saved/rebuilt control-workbook caches are explicitly
redirected into the pair's owned temporary directory; they cannot silently keep
processed XLSX files in a global cache when temporary output was selected.
If process termination or evidence/archive publication cannot be confirmed,
owned working files are retained and the report records the failure. This is
deliberate evidence preservation, not a successful temporary-output cleanup.

## SV3. Processing stages and matching

Processing is an ordered list of explicitly configured programs. A stage
declares its input granularity (file, directory or archive), output granularity,
argv vector, optional extension mapping, timeout, working directory, environment
and dependency files. The supported path placeholders are `{input}`, `{output}`,
`{output_dir}` and `{output_file}`. Arguments are passed without shell splitting.
An explicitly selected shell/interpreter remains the selected program.

File stages admit at most `max_processes` simultaneous invocations. A directory
or archive stage runs once and owns its own internal scheduling. Stages execute
in order. Only successful expected workbook outputs are passed to the next stage;
extension changes preserve relative stems. Ambiguous output paths are rejected.
Arbitrary renaming, one-to-many workbook conversion and detached background
services are outside this contract.

Excel Shrink is an optional preset using its actual cleanup contract: one XLSX
or a directory as input, and a directory as the second positional argument.
It does not receive corpus ZIPs through the ordinary cleanup command. The preset
bounds Shrink's internal workers when the outer pipeline parallelizes files.

Machine-readable command descriptions exist, for example CWL's
[CommandLineTool schema](https://www.commonwl.org/v1.2/CommandLineTool.html) and
the [Boutiques descriptor model](https://pmc.ncbi.nlm.nih.gov/articles/PMC6007562/).
They require explicit metadata supplied for a tool. Excel Shrink now exposes
`--describe-cli`: names, help, scalar types, enum choices and defaults derived
from its actual Clap definition. The bundled preset uses that same schema to
generate typed fields, including a visible Rayon-thread field, while preserving
the explicit argument array. This schema describes the bundled Excel Shrink
version, not an arbitrary executable selected by the user. Generic processors
retain the argv editor. Arbitrary help-text inference and CWL/Boutiques import
are not implemented.

Pairing tries, in order, matching relative filenames, unique basenames, matching
relative stems and unique basename stems. Ambiguous matches stay explicit;
there is no arbitrary first-file choice. Missing B files become failed/missing
records. Extra B files are recorded as unmatched candidates. Workbook content
hashes, rather than these names alone, govern later result reuse.

The current renderer accepts XLSX on both sides. `baseline_after_step` explicitly
selects the output of a one-based intermediate conversion step as A. For an
XLS-to-XLSX-to-Shrink chain, a value of 1 evaluates Shrink against the converter's
XLSX output. It **does not visually validate the XLS-to-XLSX conversion**. The
report records the selected baseline; no implicit conversion equality is claimed.

`continue_on_error` allows successful outputs and matched pairs to proceed after
ordinary process failures or missing outputs. Each invocation records its state,
exit status or timeout, expected outputs, stdout and stderr. Incomplete runs
return a failing CLI status even if some pairs succeeded. A reported visual
difference is an observation, not a process failure. Unconfirmed process cleanup
always stops further dispatch, even when continuation was requested.

## SV4. Processing cache

Persistent intermediate results are opt-in through `stage_cache_dir`. The key
combines the stage contract, program content, declared dependency-file content,
input workbook contents and explicit `processing_cache_key` for dependencies
not discoverable from argv. Directory stages also include directory sidecars;
archive stages include the original archive bytes when passed directly.
Environment-dependent programs require an honest caller-maintained key.

Only a completely successful stage publishes an immutable cache entry. Every
output, including sidecars visible to a subsequent directory stage, is checked
by length and SHA-256 before reuse. Added or missing files also invalidate the
entry. Corruption is
an error, not an automatic successful cache hit. Publication is locked and the
cache is rechecked after lock acquisition. A final stage targeting a permanent
directory bypasses this cache so it can write directly without an extra copy.

Reuse is at complete-stage granularity. This patch does not resume a partially
finished file stage or mutate an old run in place. Start a new report run with
the same cache configuration to reuse successful stages.

## SV5. Deliberately restricted reuse of visual observations

Visual reuse is independently enabled with `reuse_from`. It does not require
retaining processed workbooks from the previous run. The old report must have a
matching validator signature: capture policy, execution strategy, validator
source identity, runtime capture scripts, capture options, platform and explicit
`environment_key`. Record a nonempty environment key in a baseline intended for
future reuse, including relevant Excel/font/rendering environment changes.
Processor identity is deliberately separate, allowing evaluation of a new
processor against earlier validation evidence.

The current selection controls named OOXML categories: worksheet XML, workbook,
styles, shared strings, relationships, drawings/media, charts/chart sheets and
remaining parts. Exact extra member names are also supported. At least one
category or name must be selected. Fresh baseline runs default to recording all
categories, allowing a later narrower request.

For **both A and B**, the runner hashes selected members' uncompressed content
into a name-sorted map. ZIP entry order, compression, archive timestamps and
unselected members do not decide reuse. Member addition/removal is observable;
two absent optional members match as absent. Duplicate member names are errors.
This is byte-content comparison of selected parts, not XML canonicalization.

Previous fingerprints must cover the requested selection. Their receipt digest
and the complete comparison evidence bundle must validate. Only prior completed
or explicitly reused records qualify. Reused records keep the old observations,
source attribution and the current selection; they are labelled `reused` and
state that no new visual comparison occurred. A selected-parts match is **not a
general proof of visual equivalence**. Omitting a relevant part is the user's
scientific evaluation decision.

## SV6. Run state, crash evidence and bounded monitoring

The coordinator advances through Preparing, optional Processing, Pairing,
Validating and Publishing before Completed. Any active phase can fail. The
scientific run status separately distinguishes completion with limitations,
completion with errors and failure. Standalone pairs are validated sequentially;
the existing corpus runner retains its established group parallelism.

`run.json` is a small phase/configuration header during work. Each workbook has
an atomically replaced `records/<id>.json` receipt. The full pair index is written
once during final publication, instead of rewriting every completed workbook on
each event. `progress.json` contains counters and at most 20 recent records;
`events.jsonl` appends phase transitions, not every capture or heartbeat.
Process logs stream to disk and are not accumulated in a browser response.

An interrupted header can be reconstructed by reading its pair receipts once
when opened, exported or used as a reuse source. Running/failed receipts do not
become successes. Completed receipts can be reused in a new compatible run.
Live polling reads compact progress plus current telemetry. It does not reread
the entire historical receipt collection every second.

The existing Observatory additionally stops invisible animation, disposes hidden
historical views, bounds diagnostic and snapshot caches, and requests a live
group projection once the paged workbook index is available. These are display
and resource changes; they do not prune scientific evidence. See the measured
limits and unverified long-run hypotheses in
[Observatory resource behavior](../visual-validation/observatory-long-runs.md).

## SV7. Saved and static presentation

The live UI links each published workbook as soon as its pair receipt is complete.
A saved report uses the same image-region model and pixel inspector as the
existing Observatory. The site provides a paged/searchable workbook index,
comparison images, difference masks and available scientific context. Detailed
server-generated formula exploration and live process controls are not recreated
in the static site.

Export verifies evidence, copies only portable report material into a new
directory, and embeds the JSON needed for viewing. It works from `file://` or a
static server with no backend and no Excel installation. Export omits workbooks,
executables, raw telemetry and process logs. Metadata may still contain source
paths, configured argv or environment values; publication is not anonymization.
The original report retains those diagnostics. See
[portable reports](../visual-validation/portable-reports.md) for the bundle
integrity and diagnostic image-size limits.

## SV8. Preserved selection policy and explicit remaining scope

No byte-parser, conditional-formatting selector, state-selection proof, chart
coverage rule, adaptive partition or scientific pixel verdict is replaced here.
The existing 128-by-16 grid is already a candidate index followed by measured
adaptive merging and subdivision. The current comparison already caches PNG
and normalized RGBA fingerprints. A new universal tighter-capture or client
hash-only mode has not been introduced without native evidence for its tradeoff.
Capture options are fixed for a standalone run; no mid-capture policy mutation
is exposed. Existing legacy controls retain their own scope.

Remaining work includes arbitrary archive formats, visual validation against
legacy XLS originals, per-invocation stage resumption, independent distribution,
parallel standalone comparison admission, richer live scientific controls and
native long-duration resource measurements. These limits do not change the
coverage stated in an existing targeted report.

Permanent regression tests cover pairing ambiguity, selected-part identity,
cache integrity, interrupted receipts, process failures, report portability and
the two CLI entry points. They protect enduring behavior rather than only the
mechanics of this refactoring. No temporary refactoring-only test suite remains.
Rust/native tests were not executed in the SCE environment; supplied tests are
for the local build agent. Existing Rust unit tests now belong to the library
target (`cargo test -p excel-shrink-cli --lib`), while CLI integration tests
exercise the two real binaries.
