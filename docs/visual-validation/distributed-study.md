# Distributed corpus study on Windows

Use one local study directory per machine. Each worker knows all three corpora,
but executes only its assigned corpus and size partition. GOTHAM collects the
result packages into the combined report.

For the normal Windows workflow, use the VS Code task **Excel Shrink: Korpuslauf
konfigurieren**. Its shared corpus catalog, local ZIP migration and result
handoff are documented in [the corpus launcher guide](corpus-launcher.md).
The direct CLI examples below remain available for scripted assignments.

Prepared bundles in `prepared/` remain available after comparison. Each contains
the comparison baseline (`original.xlsx`), all three cleaning variants, and
their measurements. New bundles also include `preparation.json` with source,
conversion and output hashes. If baseline repair changed the converted workbook,
`converted.xlsx` preserves the conversion before repair; otherwise the metadata
points to `original.xlsx`. Keep these bundles when moving results between machines.

## Live controls and progress

With `--resident-capture --control-room`, the group target is controlled in the
Observatory. A group is one original workbook and its cleaning variants, plus
any required whitespace controls; it is not one Excel process.

Setting the target to **0** stops admission of new workbooks, including new
preparations. Work already admitted finishes through comparison and necessary
controls, then the runner waits until the target is raised. A preparation already
underway when zero is requested also finishes its comparison before pausing.
`--prepare-only` still stops after preparation. Starting with
`--comparison-workers 0` requires the control room; initial archive discovery
and indexing happen before this pause. No completed records are discarded.

Zero lifts a separate capture pause so that admitted work can drain. The capture
pause control is unavailable while the target is zero. Reducing a positive target
also stops further admission but never kills active work. Interactive groups close independently after completion;
`EXCEL_SHRINK_SHARED_CLEANUP=1` restores shared cleanup for diagnosis. **Working**, **waiting for cleanup**, and **open**
counts distinguish those states.

The central **Worker-Übersicht** lists active groups without traversing historical
results. It includes the Rust workbook coordinator and known Excel clients, their
process identity, observed phase, heartbeat, last real progress and bounded recent
events. A heartbeat is never counted as successful work. Unknown internal Excel
activity stays unknown; long waits alone do not trigger a failure or restart.
The compact JSON view is available at `/api/workers`; `/api/state` includes the
assignment's `progress` snapshot.

Progress counts unique source workbooks within this machine's selected corpus
and size partition. Skips and pending preparation are separate from completed
comparisons; cleaning profiles and whitespace controls do not multiply the count.
Existing results contribute to totals, not this invocation's speed. The display
shows completions per minute/hour over the last 15 active minutes and a run
average. Fully drained zero-target pauses are excluded. Capture pauses with
already-running groups retain wall time so late completions cannot inflate speed.

The remaining-time estimate uses recent completion rates and includes outstanding
preparation when its rate is available. It remains unavailable with too few
samples, missing preparation throughput, limited stage-only runs or during a
pause. File sizes and concurrency can change, so this is a current-rate estimate,
not a promised deadline. Old runs without the new live snapshot show no invented
throughput.

Each sheet card has **fit width**, **fit height** and **fit sheet** buttons shared
by its four profiles. Ctrl+wheel zooms at the pointer; Shift+wheel scrolls
horizontally. Ordinary wheel motion follows actual overflow. Manual navigation
disables camera following, and each workbook/sheet retains its viewport.

## Assignment

| Machine label | Corpus | Selection | Workbooks before resume filtering |
| --- | --- | --- | ---: |
| `ENRON-LAPTOP-1` | Enron | Smallest quarter, `1/4` | 5,218 |
| `ENRON-LAPTOP-2` | Enron | Next quarter, `2/4` | 5,218 |
| `ENRON-TOWER-1` | Enron | Next quarter, `3/4` | 5,218 |
| `ENRON-TOWER-2` | Enron | Largest quarter, `4/4` | 5,218 |
| `GOTHAM` | Data.gov | All | 9,117 |
| `DESTATIS-1` | Destatis | All | 1,182 |

The full study has 31,171 workbooks. The machine labels are readable aliases;
actual machine identity and hardware are recorded separately.

`--size-part K/N` orders the **entire original corpus** by uncompressed XLSX/XLS
file size, then member name, then archive index. It takes the rank interval
`floor(count*(K-1)/N)..floor(count*K/N)` before applying resume or priority rules.
`1/N` contains the smallest files. Equal sizes may cross a boundary: the name and
archive index make the split repeatable. This divides file counts, not total bytes
or estimated execution time. Small ZIP members can still require substantial
memory when Excel opens them.

`--order size-asc` processes the selected files from smallest to largest;
`--order size-desc` starts with the largest remaining files. Both use the original
uncompressed XLSX/XLS size, with name and archive index ascending to break ties;
unreadable sizes come last. This changes the preparation and comparison queues,
including resumed prepared files, without changing quarter membership or repeating
completed primary comparisons. `--limit` applies after ordering pending work.
The default `--order priority` retains the existing order: previously problematic
files first, followed by archive order. Missing whitespace diagnoses are still
completed before new primary work.

## Prepare each machine once

Use native Windows with desktop Excel, PowerShell, the .NET 8 SDK, Rust and its
Windows build tools. Check out the same reviewed commit on every machine. Keep
the complete repository, including the Office capture assets, and build in its
final local location; the executable uses that checkout for its runtime assets.

Run these commands from the repository root in PowerShell:

```powershell
cargo build --release -p excel-shrink-cli
New-Item -ItemType Directory -Force .tmp | Out-Null
Copy-Item -LiteralPath docs/visual-validation/portable-study-request.example.json -Destination .tmp/distributed-study-request.json
```

Copy the template only during initial setup. Edit the three `url` fields in the
local `.tmp/distributed-study-request.json` to point to the local ZIP copies.
An Enron worker needs only the Enron ZIP; absent paths for inactive corpora are
allowed. Their known totals remain in the report, and they are not downloaded.
The template contains no private access URL or credentials.

Keep every `id` unchanged. These are the canonical identities of the existing
study, independent of local drive letters. Use identical original ZIP archives
on all machines; do not unpack and repackage them, since archive indices are part
of workbook identity. A ZIP available locally takes precedence over the optional
known `total_workbooks` in the request.

## Start or resume the six assignments

Run only the command assigned to this machine. Repeating it resumes the same local
study. Completed results remain in place; adding `--retry-skipped` would explicitly
request retries and is not part of these commands. Keep the PowerShell session
open while it runs; a standalone PowerShell window allows VS Code to be closed.

Add `--hide-excel` to any assignment command when the machine is also being used
for other work. Resident Excel starts with its windows hidden and keeps them
hidden during comparison. This applies to originals,
shrink variants, and on-demand whitespace controls. Excel remains on the
interactive desktop because its Office.js runtime can stall on a private desktop.
The CLI records this choice in the run metadata and resident metrics (`hide_excel`).
Without the flag, the existing interactive
startup followed by minimization remains in use. Classic comparisons (including
fallback retries) already use a private desktop. COM opening measurements and
recalculation already run invisibly. Worker counts and memory limits still apply;
hidden-mode concurrency on each target machine should be assessed separately.

Laptop 1, 8 GiB RAM (constrained profile, one source workbook at a time):

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '512'
$env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = '512'
$env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB = '1536'
$env:EXCEL_SHRINK_SYSTEM_RESERVE_MIB = '1536'
$env:EXCEL_SHRINK_HELPER_RESERVE_MIB = '512'
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 1/4 --machine-label ENRON-LAPTOP-1 --order size-asc --resident-capture --adaptive-concurrency --comparison-workers 1
```

Two selected files completed resident comparison on GOTHAM with these limits,
without a fallback: measured Excel-job peaks were 1.306 and 1.313 GiB, and Rust
worker peaks were 173 and 91 MiB. Behavior on the actual 8 GiB laptop remains
unverified. These limits may reject a memory-demanding workbook even in the
smallest quarter. A recoverable resident
failure gets at most one sequential paired retry; if that also exceeds its
limits, the result remains a reported skip. File size alone cannot guarantee that
every workbook will fit. The reserve overrides apply only to this profile.

Laptop 2, 32 GiB RAM:

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '4096'
$env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = '2048'
$env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB = '8192'
Remove-Item Env:EXCEL_SHRINK_SYSTEM_RESERVE_MIB -ErrorAction SilentlyContinue
Remove-Item Env:EXCEL_SHRINK_HELPER_RESERVE_MIB -ErrorAction SilentlyContinue
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 2/4 --machine-label ENRON-LAPTOP-2 --order size-asc --resident-capture --adaptive-concurrency --comparison-workers 4
```

The 32 GiB profile uses the default 1024 MiB system reserve and 1 GiB helper reserve
per group. Four workers is an upper limit, not a starting batch size.

Enron tower 1 (retain its reviewed memory settings, or the defaults):

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 3/4 --machine-label ENRON-TOWER-1 --order size-desc --resident-capture --adaptive-concurrency --comparison-workers 8
```

Enron tower 2 (retain its reviewed memory settings, or the defaults):

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 4/4 --machine-label ENRON-TOWER-2 --order size-desc --resident-capture --adaptive-concurrency --comparison-workers 8
```

GOTHAM continues Data.gov with the largest remaining files first:

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '8192'
$env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = '4096'
$env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB = '32768'
Remove-Item Env:EXCEL_SHRINK_SYSTEM_RESERVE_MIB -ErrorAction SilentlyContinue
Remove-Item Env:EXCEL_SHRINK_HELPER_RESERVE_MIB -ErrorAction SilentlyContinue
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Data.gov --machine-label GOTHAM --order size-desc --resident-capture --adaptive-concurrency --comparison-workers 8
```

The default system reserve is 1024 MiB on all machines. Existing explicit
machine settings remain in effect. The Observatory's **Free RAM reserve** control
changes the reserve for active and future capture sessions.
Rust and Excel allocation limits remain separate settings.

Machine 6 handles Destatis (retain its reviewed memory settings, or the defaults):

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Destatis --machine-label DESTATIS-1 --order size-asc --resident-capture --adaptive-concurrency --comparison-workers 4
```

The worker counts are upper limits. Resident mode defaults to eight groups;
an explicit `--comparison-workers` value replaces that default without a fixed
application ceiling. For example, `--comparison-workers 24` permits up to 24
groups, subject to memory admission checks. Each adaptive wave starts one group.
Once its primary Excel clients are connected and fresh memory measurements are
available, another group may start while the earlier group is still comparing.
The same decision repeats after the newest group's primary clients are ready;
it does not wait for the whole workbook or its whitespace controls to finish.
Preparation remains serial. On the towers the first group uses the largest
remaining file; the laptops start
with the smallest files in their assigned quarter. A smaller source file is not
a guarantee of a smaller Excel memory footprint.

The estimate considers the live peaks of admitted groups and the last eight
fully measured groups. A group that shared byte-identical variants is scaled to
allow six physical clients (four primary workbooks and two controls). The
estimate adds 50% and 1 GiB, rounds up to whole GiB, and never exceeds the
configured resident limit. The chosen limit is enforced on that group's Excel
job. Rust retains its full configured limit, COM retains its own limit, and each
group additionally reserves 1 GiB for helpers by default. The default system
reserve is 1024 MiB. `EXCEL_SHRINK_HELPER_RESERVE_MIB` and
`EXCEL_SHRINK_SYSTEM_RESERVE_MIB` override these reserves for an explicitly
configured machine, as in the constrained 8 GiB profile above. With the control
room enabled, `system_reserve_mib` in its controls is the live setting. Both free RAM and
commit headroom must accommodate the next group, remaining budgets of active
groups, and this reserve. Existing groups keep their assigned limits. Physical
RAM admission credits only fresh private working-set measurements of the
resident job; shared pages and private committed memory are not substitutes.
When that measurement is unavailable, the full budget remains reserved.
Fresh current committed job memory, including Excel's child processes, is used
separately for remaining commit budgets. Rust and helper physical budgets stay
fully reserved. An observed live peak is not a promise
that a group will never allocate more memory later.

If another application consumes the available memory, the runner waits between
waves without holding Excel processes. Limits that cannot fit even one group on
the machine are rejected with a configuration error. A failure under a reduced
Excel limit resets calibration and receives one sequential paired retry, except
for known unresolved drawing boundaries. The original attempt remains visible;
incomplete measurements never justify increasing concurrency.

For an existing checkout with prepared bundles, retain any separately reviewed
`--reuse-prepared-from FULL_SHA256` arguments from its current launcher. A new
executable does not automatically accept preparations from another build. The
canonical IDs allow GOTHAM to retain results from its earlier private request.

Restarting a study retries previously skipped workbooks in the selected partition
once. Successful comparisons remain complete, and compatible preparations are
reused. Previous failures remain in the attempt history. To leave skipped entries
complete for a particular CLI invocation, pass `--no-retry-skipped`.

Before opening Excel, the runner checks whether all three prepared profiles have
equivalent package contents to the prepared comparison baseline. This requires matching contents for
every ZIP member, not just the worksheet XML. The only permitted content change
is removal of proven redundant defined names in `xl/workbook.xml`; matching file
sizes alone are not evidence. ZIP compression and packaging metadata can differ.
If any profile fails this check, the normal Excel comparison still runs.
The baseline can already contain the existing workbook repair or XLS conversion;
it is not necessarily byte-identical to the source file. Source and baseline
fingerprints remain separate, and the report discloses repaired baselines.

A successful package check completes the workbook as unchanged without Excel
captures, value checks, opening timings or whitespace controls. The report labels
these records **Unveränderte Paketinhalte – Excel-Vergleich ausgelassen** and
shows their share of the unchanged total separately. They retain package evidence
and machine provenance in result exports but do not enter the visual-verification
CSV ledger. Existing completed visual results are retained when resuming.

Before the first real run, append `--selection-only` to the assigned command to
inspect the selected files without shrinking or starting Excel. It writes
`.tmp/regression/study/selection-manifest.json` and a content-addressed copy under
`selections/`. The manifest records the full ordered catalog, sizes, selected rank
bounds, source fingerprint, catalog hash and requested processing order. The
catalog itself remains ascending so the partition ranks stay comparable across
orders. `--selection-manifest PATH` writes
an additional copy. A partition's completed work is never presented as completion
of the entire three-corpus study.

## Live control room

For interactive admission, replace `--adaptive-concurrency` with `--control-room`.
The two modes are mutually exclusive. Keep the machine's corpus selection, memory
limits and reviewed `--reuse-prepared-from` arguments. For example:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Data.gov --machine-label GOTHAM --order size-desc --resident-capture --control-room --control-room-port 8771 --comparison-workers 2 --startup-timeout-seconds 300 --workbook-timeout-seconds 1800
```

Open `http://127.0.0.1:8771/`. `--comparison-workers` sets the initial group target;
the dashboard can raise or lower it during the run, without a fixed eight-group
ceiling. Increasing it admits additional groups without waiting for the current
group's startup or estimating another group's RAM. Lowering it prevents further
admission until the target permits it; existing comparisons finish normally.
Per-process memory limits and the emergency system-memory guard still apply.
Completed interactive groups close independently and free their admission slot.
Cleanup process identities and timestamps are retained for peer-failure diagnosis;
see [cleanup isolation](cleanup-isolation.md). Preparation remains serial.

Capture pause stops dispatching new Office.js jobs. A job already running may
finish and publish its tile; new Excel instances may continue opening. Each group
waits for its primary clients, then captures a corresponding batch across its
profiles before advancing. Clients upload each image as it becomes available;
full-resolution host comparison
determines equality and retained previews. The dashboard displays the actual job
rectangles, including partial charts, and follows or pans over large sheets.
Recoverable failures can still use the existing sequential classic fallback. The
group is explicitly marked as a classic retry; resident tile progress does not
describe that separate attempt.

`--startup-timeout-seconds` supplies the initial active-opening timeout and
`--workbook-timeout-seconds` the initial live job timeout. Both can be changed in
the dashboard. Time waiting for another Excel launch does not spend the active
opening timeout. A user pause does not consume a fixed whole-session deadline.
Closing the browser does not pause or stop the runner. Closing the runner ends
its owned Excel work; the local control server retains the completed history.

The same controls are available to scripts and agents on loopback:

```powershell
Invoke-RestMethod http://127.0.0.1:8771/api/state
Invoke-RestMethod http://127.0.0.1:8771/api/control -Method Post -ContentType application/json -Body '{"target_groups":4,"paused":false}'
Invoke-RestMethod http://127.0.0.1:8771/api/control -Method Post -ContentType application/json -Body '{"open_timeout_seconds":900,"job_timeout_seconds":1800,"equal_preview_width":256}'
```

`GET /api/groups/{id}` returns the selected group's sheet, tile, image and launcher
states; `GET /api/state` lists its ID. Equal preview widths are `128`, `256`, `512`,
`1024`, or `0` for original resolution. Different tiles retain full resolution.
Preview changes apply as new tiles complete. Control snapshots and group history
live under `<output-root>/control-room`; scientific result records retain their
existing locations and export format. The server checks its host source version
on resume. After a host-code update, stop the older control server or choose a
different `--control-room-port`; the CLI will not terminate an unknown server.

The curved overview contains every sheet, with four profile mosaics per sheet.
Transparent outlines indicate planned captures. Images occupy their measured
pixel coordinates; tall sheets can be panned and zoomed. Click a sheet to focus
it, or a capture region to inspect matching areas across profiles. The minimap
previews sheets on hover. Camera following can be disabled while inspecting
history; navigation buttons find the next differing region, sheet or group.

Capture size (`capture_tile_megapixels`, integer 1–32, default 32), batch size
(`capture_batch_tiles`, integer 1–16, default 1) and `hide_excel` apply to the
next admitted group. Each attempt records those settings. The batch is also
bounded to 64 MP per physical client. Actual capture jobs remain full-width,
guarded row bands so overflow text and charts are rendered consistently. Large
images are subdivided into explicitly labelled inspection regions, with exact
per-region pixel comparisons; these regions are not extra Excel capture jobs.
The throughput display counts real uploaded pixels and bytes, including repeated
or overlapping pixels, rather than estimating from displayed thumbnails.

The process list shows identity-checked Excel PIDs, actual launch visibility and
current work. Hearts pulse on real worker heartbeats and fade with their age.
A live process without a recent heartbeat remains distinguishable from a dead
process, including while Excel opening stalls Office.js. Hidden launches can be
observed entirely through this dashboard; whether an Office build executes its
add-in reliably while hidden still needs a representative native test.

The capture signal lane shows worksheet ranges as spatial miniatures. New ranges
are blue, the preceding batch is gray, and the older batch is faded. Expand
**Worksheet capture area** in the worker overview, or select a worker, to compare
the last three batches on the same coordinate grid. The four Excel worker cards
below the Atelier show this area directly. Each tile opens its exact capture and
pauses following; **Follow captures** resumes live navigation.

Capture timestamps mark receipt and validation by the host, not the instant Excel
created the image. Command IDs identify actual batches. Missing IDs use labeled
one-second visual groups; crowded timeline glyphs show an additional-group count.
Only observed ranges are drawn, and missing coordinates remain explicit. The
60-second lane expires normally, while the last spatial context stays available
during quiet periods. This context holds at most 256 observed captures per worker
and resets with its process or attempt; it is not a complete capture archive.

```powershell
Invoke-RestMethod http://127.0.0.1:8771/api/control -Method Post -ContentType application/json -Body '{"capture_tile_megapixels":4,"capture_batch_tiles":4,"hide_excel":true}'
Invoke-RestMethod http://127.0.0.1:8771/api/groups/GROUP_ID/restart -Method Post -ContentType application/json -Body '{"reason":"Accidental cell edit"}'
```

A restart always repeats the whole workbook comparison, even when requested
from one instance. The coordinator stops issuing jobs, waits for the current
wave's peers to finish, closes the owned Excel instances together, and puts the
replacement first in the queue. If captures are paused, resume them to let those
peers finish. Completed workbooks in the current corpus/size selection can also
be queued; their shrink variants are rebuilt only if the prepared ZIP is missing
or no longer matches the accepted source, build and profile settings.
Every attempt uses separate image/evidence paths and archives the previous
record. A new attempt cannot silently mix images from an interrupted one.
`GET /api/commands/{id}` exposes the durable request's state and final result.

## Memory and timing interpretation

The Rust worker defaults to a 4,096 MiB process limit. An existing reviewed
8 GiB configuration, such as GOTHAM's, can be retained in that PowerShell session:

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '8192'
```

This caps the Rust worker rather than preallocating memory; admission budgets
reserve that full limit. It does not cap all processes combined. Excel has a
separate limit and the operating system, Excel,
WebView and other applications also need RAM. Do not apply the tower setting to
small laptops without checking their available memory. Record any changed limits.

Compare original and shrunk open times **within the same paired measurement on
the same machine and run**: `original_seconds / shrunk_seconds`. A value of `2`
means twice as fast in that measurement. Absolute times from different machines
are not interchangeable. Ratios can also differ with CPU, storage, caching,
Excel version, workbook features and memory pressure; equal ratios are not an
assumption of this experiment. Size-based assignment gives machines different
workloads, so a cross-machine ratio comparison is not a randomized hardware
benchmark.

Peak memory is not guaranteed hardware-independent either. Distinguish private
commit from working set and retain the measured values, limits and environment.
The machine manifest records CPU, core counts, installed RAM, Windows and Excel
information when available. Missing metadata remains explicit. Preparation,
primary comparison and supplemental whitespace diagnosis are attributed
separately; resuming on another machine does not relabel historical measurements.
Legacy results without provenance remain unknown.

The run manifest identifies the actual executable by SHA-256. Its
`checkout_git_commit` and `checkout_dirty` describe the runtime checkout at start;
they do not claim which commit produced an arbitrary copied executable. Build
from the clean final checkout and keep its runtime assets unchanged during a run.

## Resident visual checks

The resident mode keeps the original and its cleaning variants open for visual
comparison. Enable it explicitly on a machine with enough available RAM:

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '8192'
$env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = '4096'
$env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB = '32768'
Remove-Item Env:EXCEL_SHRINK_SYSTEM_RESERVE_MIB -ErrorAction SilentlyContinue
Remove-Item Env:EXCEL_SHRINK_HELPER_RESERVE_MIB -ErrorAction SilentlyContinue
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Data.gov --machine-label GOTHAM --order size-desc --resident-capture --adaptive-concurrency --comparison-workers 8
```

Retain the existing `--reuse-prepared-from` arguments when resuming. Without
`--resident-capture`, the default remains sequential paired capture with one
worker. Resident mode defaults to eight groups. `--comparison-workers` accepts
any positive integer representable by the platform's native count type; an
explicit value replaces the default, including values above 16 or 255. It
controls source workbooks, each of which can have several Excel instances.
More than one worker and `--adaptive-concurrency` require `--resident-capture`.
Without the adaptive flag, the requested fixed batch size is still limited by
available memory; no calibration changes the configured allocation limits.

Workers first send tile dimensions and PNG SHA-256 digests. Only differing tiles
are transferred for exact pixel comparison. Certified byte-identical workbooks
can share a capture; volatile calculations and other runtime-dependent inputs
prevent this reuse. Value checks still run for every cleaning profile.

A whitespace control compares the original against the same cleaning profile
with whitespace deletion disabled. It captures only tiles that differed in the
primary comparison, or reuses those exact captures from an identical open
variant. The report explicitly marks this limited control scope. An incomplete
control remains unresolved.

Adaptive waves grow during comparison until the requested ceiling, available
memory, the first finished group, or the end of the queue stops admission. They then drain together;
finished groups are not replaced within that draining wave. Resident Excel
sessions are retained until every admitted peer finishes or fails, protecting
Office/WebView resources shared between Excel processes. Nonadaptive runs use
fixed batches. Completed results are checkpointed before cleanup waits.

While a newly admitted group prepares and opens its primary clients, the
coordinator records bounded pause time for existing peers. Their processing
deadlines exclude these opening pauses, because Excel startup can stall Office.js
in another group. The newly opening group's own deadline, parent liveness,
memory guards, and cleanup deadlines remain active. A stale pause counter cannot
keep a stuck comparison alive indefinitely.

Without adaptive concurrency, admission adds each group's configured Rust limit
and Excel limit, then keeps
1 GiB for the system by default. Excel's resident-session limit defaults to 16 GiB; a larger
configured COM limit also counts. With an 8 GiB Rust limit and the default
16 GiB resident limit, two groups need 49 GiB available and three need 73 GiB.
GOTHAM's command above instead permits a 32 GiB resident limit and uses adaptive
admission. Unknown available memory falls back to
one worker. These checks do not lower either allocation limit.

Adaptive concurrency uses the observed peaks to choose an enforced Excel session
limit for each newly admitted group, bounded by the configured resident limit. It retains the
full configured Rust and COM limits, reserves another 1 GiB per group for helpers,
and keeps 1 GiB for the system by default. The constrained laptop profile uses
the explicit reserve overrides listed above. Admission checks the sum of these effective
limits against both available physical RAM and commit capacity. It holds pending
work when even one group cannot safely fit. The first calibration may also need
a lower Excel limit on a small machine; a laptop with 16 GiB cannot reserve a
4 GiB Rust worker plus a 16 GiB Excel session and system memory simultaneously.
The laptop commands therefore provide separate memory settings suited to their
reported RAM; they do not promise that every selected workbook fits those caps.

The requested order and concurrency settings remain in the run metadata. Each
record retains wave size, admission reason, effective and reserved limits,
capture method, session memory and machine provenance. These are throughput runs for visual regression; opening times under
concurrent load should not be pooled with an isolated performance experiment.

If a primary resident check hits an image-budget error, a taskpane restart,
Excel's `InvalidOperationInCellEditMode` error, or its reduced adaptive Excel limit, it
gets one sequential retry through the existing paired path after the batch has
finished. The retry uses the full configured budgets and must also fit available
memory; otherwise it remains pending. The original failure and retry mode remain
in the record. Other errors are not automatically retried.

## Export completed work and merge on GOTHAM

The transfer commands use the study directory's exclusive lock. Run them between
study invocations, when that directory is not being used by a runner or worker.
They do not launch Excel. For example, on laptop 1:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --report-only --export-results .tmp/enron-laptop-1-results.zip
```

Use a fresh package name for each export; an existing package is not overwritten.
Repeat on the other machines with distinct filenames. Copy the resulting ZIPs to
GOTHAM, for example into `.tmp/incoming/`.

The package contains completed workbook records (including reported skips), their
referenced PNG evidence, and the required machine/run metadata. Incomplete work,
prepared XLSX bundles and private request files are excluded. Each payload entry
has a size and SHA-256 recorded in the package manifest. Preserve the package as
part of the research evidence.

Historical records without a source catalog fingerprint remain in the local
report but are excluded from portable packages. Their identities are listed in
`excluded_missing_source_fingerprint` in the package manifest and receipts.
The exporter does not invent an archive match for these historical measurements.

Import several packages and rebuild the report in one command:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --report-only --import-results .tmp/incoming/enron-laptop-1-results.zip --import-results .tmp/incoming/enron-laptop-2-results.zip --import-results .tmp/incoming/enron-tower-1-results.zip --import-results .tmp/incoming/enron-tower-2-results.zip --import-results .tmp/incoming/destatis-1-results.zip
```

Import verifies payload hashes, workbook identities, the requested corpus
whitelist and available source catalogs. An identical repeated import is
idempotent. Different results for an existing identity are retained as conflicts
with an import receipt; the existing result is not silently overwritten. Inspect
`.tmp/regression/study/imports/` before resolving any conflict. The HTML dashboard
and `diagnosis.json` are rebuilt from the combined durable records, with machine
groups and the separately attributed phases available for review. Then resume
GOTHAM with its Data.gov command above.

This is deterministic work assignment and result exchange, not a shared task
queue. When a machine finishes its assignment, arrange a new non-overlapping
assignment before starting another worker on the same corpus.
