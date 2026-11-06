# Observatory resources during long runs

These display controls were backported from `codex/sheetscope-validation` at
`a7445eb`. The standalone SheetScope workflow remains a separate development.

## Resource ownership

| Component | Bound or lifecycle |
| --- | --- |
| Legacy Three.js scene | One animation callback while the page and scene are visible. Targeted mode suspends the hidden legacy scene. |
| Historical comparison cards | Only active and explicitly displayed historical cards retain canvas, observer and DOM trees. Their report records support reopening. |
| Main browser poll | One second during a run, five seconds after completion, ten seconds in a hidden tab. |
| Live state response | Every active group, the selected workbook and eight recent inactive summaries. The paged workbook index owns historical browsing. |
| Parsed display snapshots | At most 512 entries and a 32 MiB JSON accounting budget per host. Metadata changes invalidate entries, with a forced reread after 30 seconds. |
| Formula responses | 64 entries and an 8 MiB JSON accounting budget. At most 16 pending requests. |
| Formula summary strings | 512 entries and a 256 KiB accounting budget. |
| Pixel responses | 512 entries and an 8 MiB JSON accounting budget. At most 128 pending requests, with interactive priority for the selected region. |
| Diagnostic report loading | Each diagnostic kind acquires its existing semaphore before loading the report. Waiting requests retain their identifiers. |

The JSON budgets estimate retained data rather than total process memory.
Allocator, DOM, image and browser overhead require separate measurements.
Oversized display snapshots are read normally and omitted from the cache.
Read failures discard cached entries. Deferred diagnostics carry an explicit
unavailable/deferred result.

Snapshot producers use atomic replacement. Length, modification time and
available creation time invalidate cached entries. A writer preserving all
observed metadata could leave a display stale until the 30-second revalidation.
Control requests retain their existing run and attempt identity checks.

## Scientific ownership

These caches serve display reads. Capture selection, workbook-state comparisons,
image verdicts and durable evidence reuse belong to the validation pipeline.
The complete `/api/state` response remains available to existing API clients.
`group_scope=live` is an explicit projection, with `groups_total` and
`groups_truncated` describing omitted history. Active groups are always included.

The comparison pipeline already retains immutable image fingerprints instead
of decoded bitmaps. Its telemetry coalesces ordinary publication and bounds
recent worker events. The scheduler still publishes the corpus-wide queue and
workbook index. A live projection reduces transmission and browser processing,
while queue publication and reconciliation remain measurable costs.

## Local controls on 2026-11-06

An isolated copy of the real 9,117-workbook queue was served by the production
release and the original review-branch release. Each host owned a separate
temporary root. The production comparison processes continued running.

After warming each endpoint, eight requests produced these short-run medians.

| Endpoint | Response bytes | Groups | Median request time |
| --- | ---: | ---: | ---: |
| Production `/api/state` | 3,820,627 | 9,117 | 66 ms |
| Review branch `/api/state` | 3,820,730 | 9,117 | 55 ms |
| Review branch live projection | 7,293 | 12 | 47 ms |

All four active group IDs survived the projection. This demonstrates the
payload reduction on the actual corpus metadata. It is a small frozen-queue
control rather than a long-duration responsiveness benchmark.

A separate 15-second production observation measured the Observatory server at
about 1.4 percent total CPU and 97 MiB private memory. A contemporaneous EEO1
2016 comparison process used about 12 GiB private memory while analysing a
workbook with 405 MiB uncompressed worksheet XML. The scientific parser retains
layout metadata across states and materializes change lists. An allocation
profile is required to attribute that process memory precisely.

Physical availability and commit headroom appear separately in the memory
tooltip. The current Data.gov deployment uses four comparison groups and a
1,024 MiB reserve. Browser work, native Excel rendering, scientific XML analysis,
Defender scanning and system resources need separate measurements when
diagnosing system-wide stalls.

## Verification

The source-branch review passed `cargo check --all-targets`, targeted snapshot
cache and projection tests, and the HTTP live-state pagination test. The
backport passed its own 37 Rust host tests and all 33 Python control-room
protocol tests. JavaScript regression checks cover animation ownership,
diagnostic eviction, request saturation, priority and report identity.

The protocol suite had a stale invalid-batch example of 17. The production
binary already accepts batches through 1,024, so this example altered the
fixture and caused subsequent state-preservation assertions to fail. Running
that test against the original production binary reproduced all 22 failures.
The invalid example now uses 1,025, with round-trip coverage for 1, 17 and 1,024.

Local receipts and benchmark inputs are under the deployment's temporary root
in `sheetscope-host-benchmark-release`, `sheetscope-review-*.log` and
`observatory-cache-*.log`. They are machine-local artifacts. A browser/GPU trace
is still needed to quantify the responsiveness effect of hidden-scene and DOM
lifecycle changes.
