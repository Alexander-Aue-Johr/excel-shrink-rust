# Observatory resource behaviour during long runs

This is a source review and a scoped correction, not a reproduced 24-hour
Windows failure or proof of one cause. The exported project contains no
long-duration CPU, commit, GPU or disk trace from the affected machine.

## Findings and corrections

| Source observation | Consequence | Change in this patch |
| --- | --- | --- |
| `control-room/app.js` scheduled and rendered the Three.js scene on every animation frame, including when targeted mode hides `.room` through CSS. | An invisible legacy scene still consumes browser CPU/GPU work. | One visibility-owned animation loop stops scheduling frames while the page or scene is hidden. Hidden scenes are not rebuilt or loaded merely by polling. Classic capture presentation resumes when it becomes visible. |
| Every main poll returned the entire `queue.json` group array; the browser polled every 500 ms. | Transport, JSON parsing and repeated scans grow with corpus size, even if only one workbook changes. | Targeted mode, once its paged workbook index exists, requests a live projection containing all active groups, the selected group and at most eight other recent inactive groups. The corpus remains browsable through `/api/workbooks`. Polls wait 1 s while running, 5 s after completion, and 10 s in a hidden tab. |
| Queue/index readers and comparison-registry requests repeatedly read and parse unchanged JSON snapshots. | Completed work keeps consuming disk/JSON work during active monitoring and browsing. | A per-host LRU reuses parsed read-only snapshots, with at most 512 entries and a 32 MiB accounting budget for retained JSON. File length, modification time and available creation time invalidate entries; a 30 s reread also bounds reliance on metadata stamps. Detected changes and read failures discard the cached entry. JSON payloads above 64 KiB are parsed off the asynchronous executor threads. |
| The comparison panel retained canvas, observer and DOM trees for historical cards after merely hiding them. | Up to 100 previously active comparisons retained their full display trees and received updates. | Only active and explicitly displayed historical cards keep views. Hidden cards dispose canvases/observers; their small records still permit later reopening. |
| Formula diagnostics retained complete responses in resolved promises indefinitely. Pixel diagnostics used an unbounded response map and pending list. | Visiting more workbooks or expanding long result lists accumulated diagnostics and closures. | Formula responses: 64 entries / 8 MiB JSON-size budget; formula summary strings: 512 / 256 KiB. Pixel responses: 512 / 8 MiB. Pending requests are capped at 16 for formulas and 128 for pixels. A selected pixel region can displace a queued background summary. A deferred request is shown as unavailable/deferred, never as an equality result. |
| Diagnostic endpoints loaded complete reports before waiting for their image/formula work semaphore. | Concurrent waiting requests could each retain a large report. | Acquire the existing per-kind semaphore before report loading; waiting requests retain only their identifiers. |
| System-memory display reported physical availability but no commit headroom. | Physical RAM alone did not expose a second relevant resource limit during diagnosis. | Windows `/api/state` memory includes `commit_limit_bytes`, `commit_available_bytes` and `commit_used_bytes` from `GlobalMemoryStatusEx`; the memory indicator exposes headroom in its tooltip. This is telemetry, not a new admission rule. |

The JSON budgets are accounting bounds, not measurements of total RSS. Object,
allocator, browser and image-buffer overhead is additional. Entries exceeding a
cache budget are read normally without retention. The cache changes display
reads only: it is not used as scientific evidence or as the configurable OOXML
result-reuse policy.

Snapshot producers use atomic replacement. If another writer preserves every
observed metadata field while changing the bytes, display data can remain stale
until the 30 s revalidation. Control requests remain scoped to the existing run
and attempt identifiers; the scheduler and result writers retain ownership of
scientific state.

The main state endpoint remains backward compatible: `/api/state` returns the
complete group array. `/api/state?group_scope=live&selected=<group-id>` opts into
the projection and includes `groups_total`, `group_scope: "live"`, and
`groups_truncated`. Unknown scopes and invalid selected IDs are rejected. Live
process groups are never discarded to meet a display cap. The existing paged
index owns corpus search and historical navigation, and classic mode retains
its existing full-queue navigation.

## Existing protections retained

These paths already limit resource growth and were not replaced:

- `capture_host/resident_events.rs`: a coalescing `tokio::sync::watch` revision,
  not an ever-growing in-memory event queue; reconnects reconcile durable files.
- `capture_host/resident_telemetry.rs`: 24 recent worker events. Ordinary idle
  polls and heartbeat samples do not extend that history.
- `regression/targeted_comparison/telemetry.rs`: coherent atomic snapshots,
  capture-counter publication coalesced to 250 ms, and independent process
  observations. The snapshot is replaced; it is not an accumulated event log.
- `control_room/targeted_history.rs`: every five minutes, terminal display
  mirrors with confirmed closed processes are archived, retaining 200 recent
  mirrors. Scientific evidence is preserved separately. Unconfirmed cleanup
  cannot be silently treated as closed for the sake of a history limit.
- `control-room/signal-model.js`: worker/event bounds, bounded observation
  windows and explicit eviction. The shared icon/heartbeat shaders also already
  avoid creating a separate WebGL context for each card.
- `regression/targeted_comparison/image_comparison.rs`: immutable images are
  represented by encoded and normalized RGBA fingerprints after decoding.
  Decoded bitmaps are not retained in this comparison cache.
- Pixel diagnostics already serialize decoding and persist computed diagnostics
  keyed by evidence. The new browser limits supplement that existing disk cache.

A replacement of these bounded telemetry paths by SQLite or an external
database would not remove the hidden render loop, historical DOM trees or
unbounded browser diagnostic retention. No database infrastructure is added.
Atomic result publication and crash evidence remain unchanged.

## Remaining costs and uncertainty

The legacy scheduler still writes a corpus-wide queue and workbook index.
Readers must still traverse the queue to derive process status and the index
to apply filters. A live-state projection reduces transport, not the cost of
publishing or initially reading that queue. Parsing is still necessary whenever
the source changes or exceeds the cache budget. The targeted registry still
lists retained mirrors; numerous genuinely unconfirmed attempts can exceed its
normal terminal-history size. These are further places to measure if I/O stays
high after this patch.

The current `/api/groups/{id}` detail path still reads several selected-group
snapshots, command receipts and the prepared-bundle directory. This is scoped to
the selected group, but not an incremental event stream. There is no claim that
all Observatory disk reads have been eliminated.

The system reserve defaults to zero in the exported corpus controller, and the
existing UI allows high capture concurrency and sizeable capture budgets.
Concurrent Excel processes, Office.js/WebView heaps, transient decoded images,
Windows commit pressure and GPU resources therefore remain plausible causes of
system-wide stalls. A local time series is needed to distinguish them from the
fixed polling costs. The patch preserves existing concurrency and memory-policy
choices rather than silently changing experiment settings.

For the affected Windows run, correlate the slowdown with private commit and
working set of the runner, Excel, WebView and browser, system commit headroom,
hard faults/page activity, disk queue and GPU usage. Comparing the same running
experiment with the Observatory tab closed versus open isolates browser work;
reading durable completion/cleanup records distinguishes accumulating live
Excel processes from display history. These observations should precede a
larger persistence or scheduling redesign.

## Regression coverage and verification

Permanent regression tests cover cache invalidation/eviction, bounded corpus
projection, visibility-driven render scheduling, diagnostic saturation and
report identity. They protect requested resource behaviour; they are not
throwaway refactoring-only tests.

Executed in the supplied workspace:

```text
node tests/diagnostic-cache.test.js
node tests/animation-loop.test.js
node tests/pixel-diagnostics.test.js
node tests/formula-diagnostics.test.js
node tests/model.test.js
```

The commands run from `excel-shrink-cli/office-capture/control-room` and passed.
Modified UI scripts also passed `node --check`; the modified Python protocol
test parsed successfully with `ast.parse`.

Not executed here: Rust compilation/unit tests, the loopback host integration
test, browser/GPU UI verification, Excel/Office.js/PowerShell execution, or a
long-running Windows experiment. Local validation should include the new
`snapshot_cache` tests, `live_projection_retains_active_selected_and_recent_without_expanding_with_corpus_size`,
and `ControlRoomProtocol.test_live_state_is_bounded_while_workbook_history_remains_pageable`
from `office-capture/resident-host/control_room_test.py` against the rebuilt host.
