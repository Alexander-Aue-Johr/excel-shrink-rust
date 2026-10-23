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

Laptop 1:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 1/4 --machine-label ENRON-LAPTOP-1
```

Laptop 2:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 2/4 --machine-label ENRON-LAPTOP-2
```

Enron tower 1:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 3/4 --machine-label ENRON-TOWER-1
```

Enron tower 2:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Enron --size-part 4/4 --machine-label ENRON-TOWER-2
```

GOTHAM continues Data.gov:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Data.gov --machine-label GOTHAM
```

Machine 6 handles Destatis:

```powershell
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Destatis --machine-label DESTATIS-1
```

For an existing checkout with prepared bundles, retain any separately reviewed
`--reuse-prepared-from FULL_SHA256` arguments from its current launcher. A new
executable does not automatically accept preparations from another build. The
canonical IDs allow GOTHAM to retain results from its earlier private request.

Before the first real run, append `--selection-only` to the assigned command to
inspect the selected files without shrinking or starting Excel. It writes
`.tmp/regression/study/selection-manifest.json` and a content-addressed copy under
`selections/`. The manifest records the full ordered catalog, sizes, selected rank
bounds, source fingerprint and catalog hash. `--selection-manifest PATH` writes
an additional copy. A partition's completed work is never presented as completion
of the entire three-corpus study.

## Memory and timing interpretation

The Rust worker defaults to a 4,096 MiB process limit. An existing reviewed
8 GiB configuration, such as GOTHAM's, can be retained in that PowerShell session:

```powershell
$env:EXCEL_SHRINK_WORKER_MEMORY_MIB = '8192'
```

This is a limit for the Rust worker, not a reservation and not a limit for all
processes combined. Excel has a separate limit and the operating system, Excel,
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
& .\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/study --run-corpus Data.gov --machine-label GOTHAM --resident-capture --comparison-workers 2
```

Retain the existing `--reuse-prepared-from` arguments when resuming. The default
remains the sequential paired capture. `--comparison-workers` accepts 1 through 3;
it controls source workbooks, each of which can have several Excel instances.
More than one worker requires `--resident-capture`.

Workers first send tile dimensions and PNG SHA-256 digests. Only differing tiles
are transferred for exact pixel comparison. Certified byte-identical workbooks
can share a capture; volatile calculations and other runtime-dependent inputs
prevent this reuse. Value checks still run for every cleaning profile.

A whitespace control compares the original against the same cleaning profile
with whitespace deletion disabled. It captures only tiles that differed in the
primary comparison, or reuses those exact captures from an identical open
variant. The report explicitly marks this limited control scope. An incomplete
control remains unresolved.

Parallel work runs in fixed batches. No replacement workbook starts, and no
resident Excel session is cleaned up, until all peers finish or fail. This also
protects Office/WebView resources that can be shared between Excel processes.
Completed results are checkpointed before this wait; waiting does not consume
the workbook's processing deadline. Parent loss and bounded cleanup still apply.

Admission adds each group's configured Rust limit and Excel limit, then keeps
4 GiB for the system. Excel's resident-session limit defaults to 16 GiB; a larger
configured COM limit also counts. With GOTHAM's 8 GiB Rust limit, two groups need
52 GiB available and three need 76 GiB. Unknown available memory falls back to
one worker. These checks do not lower either allocation limit.
The record retains actual batch size, reserved limits, capture method, session memory and machine
provenance. These are throughput runs for visual regression; opening times under
concurrent load should not be pooled with an isolated performance experiment.

If a primary resident check hits an image-budget error, a taskpane restart, or
Excel's `InvalidOperationInCellEditMode` error, it
gets one sequential retry through the existing paired path after the batch has
finished. The original failure and retry mode remain in the record. Other errors
are not automatically retried.

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
