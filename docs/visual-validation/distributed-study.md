# Distributed corpus study on Windows

Use one local study directory per machine. Each worker knows all three corpora,
but executes only its assigned corpus and size partition. GOTHAM collects the
result packages into the combined report.

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

The 32 GiB profile uses the default 4 GiB system reserve and 1 GiB helper reserve
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

On GOTHAM, adaptive admission keeps the default 8 GiB system reserve; the runtime
guard retains its separate 4 GiB floor. The two Enron towers and the Destatis
machine retain their existing reviewed settings or defaults until their RAM is
known. Do not copy the 8 GiB laptop's reserve overrides to those machines.

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
reserve is 10% of total RAM rounded up to GiB, with a minimum of 4 and maximum of
8 GiB. `EXCEL_SHRINK_HELPER_RESERVE_MIB` and
`EXCEL_SHRINK_SYSTEM_RESERVE_MIB` override these reserves for an explicitly
configured machine, as in the constrained 8 GiB profile above. Both free RAM and
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
4 GiB for the system. Excel's resident-session limit defaults to 16 GiB; a larger
configured COM limit also counts. With an 8 GiB Rust limit and the default
16 GiB resident limit, two groups need 52 GiB available and three need 76 GiB.
GOTHAM's command above instead permits a 32 GiB resident limit and uses adaptive
admission. Unknown available memory falls back to
one worker. These checks do not lower either allocation limit.

Adaptive concurrency uses the observed peaks to choose an enforced Excel session
limit for each newly admitted group, bounded by the configured resident limit. It retains the
full configured Rust and COM limits, reserves another 1 GiB per group for helpers,
and keeps 4–8 GiB for the system by default. The constrained laptop profile uses
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
