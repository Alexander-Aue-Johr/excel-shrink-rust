# Recorded execution history

## SheetShrink Observatory

The native Observatory records workbook, worksheet and chunk events beside the
output directory in `<output>.events.jsonl`. After a run stops, **Replay recorded
run** reads that log in cursor-based pages. Playback supports pause, restart,
seeking and speeds from 0.05× to 2×. It reconstructs state from events observed
by the selected time, including metadata and completion reports. Playback does
not invoke cleanup or write workbook outputs. The displayed clock remains the
recorded elapsed time.

Replay currently belongs to the current native server's run. Browser reloads
retain access while that server remains open. Reopening an older log in a new
server is not yet supported. A missing or malformed log produces an error.
The browser loads the full event history for replay, so its memory use grows
with the recording. Ordinary live polling retains its bounded history.

Each worksheet has a stable color across the worker timeline and Workbook Atlas.
Hovering either original or optimized columns links their work items to that
column. Hovering a work item targets the optimized column. Column bodies and
their top surfaces are hover targets. Clicking or using **Link worksheet**
pins that selection. The atlas brings the focused workbook into view. Colors
distinguish worksheets within a workbook and can repeat across a large corpus.
Identity and links use worksheet IDs, independently of color.

**Figure view** hides configuration and secondary charts. Light/dark themes and
the **Publication view** camera support screenshots. **Export figure SVG** exports
both scenes, their current cameras and any focused worksheet links as editable
vector geometry. Atlas heights use uncompressed ZIP-entry bytes. Runtime and
workbook totals describe the actual recorded run. Slower playback never changes
those measurements. A scheduling figure alone does not establish speedup or
visual equivalence of the cleaned workbooks.

Timeline and atlas share one panel, with atlas selection and pagination beneath
both charts. The panel's maximize control expands both diagrams and their
connections together. Escape or the restore control returns to the normal view.
The shared SVG export remains in the panel header when maximized.

## Implemented run-progress channel

New standalone DeltaSieve runs create `replay/manifest.json`. The orchestration
records aggregate phase, current workbook, total/completed/failed/reused counts,
difference/equality counters and bounded error text whenever its progress
publication path runs. Current names are limited to 512 characters and error
text to 2,048 characters in this derived history. Original reports retain their
full evidence. The existing Enron runner is pinned and has not been replaced
to enable this new recording path.

The clock uses monotonic elapsed milliseconds within this invocation. Every
event has a strictly increasing sequence and a nondecreasing timestamp. Each
state is a complete checkpoint, allowing independent reconstruction from a
selected chunk. This channel does not yet contain process, sheet, capture or
heartbeat observations. Those omissions are explicit in its manifest. Existing
runs without this channel have unavailable history.

Chunks are independently gzip-compressed JSON arrays containing at most 32
events. Each event is bounded to 64 KiB and each decoded chunk to 512 KiB plus
64 bytes of array delimiters. SHA-256, compressed size, event count, sequence
start and timestamp bounds are retained for each chunk. Consumers must bound
decompression and validate identities before using a chunk.

Publication writes a chunk atomically, then appends and syncs its metadata to
`chunks.jsonl`. The complete manifest is published at normal terminal handling,
including handled failures. Its `complete` flag concerns this recording channel
and does not mean that validation succeeded. A failed run retains its failed
terminal checkpoint. During recording the manifest remains incomplete; durable
chunks can be located through the append-only index. An abrupt termination can
lose at most 31 unflushed checkpoints. An incomplete trailing index line must
be treated as unavailable evidence. Historical transitions are never invented.

The aggregate source intentionally omits large comparison records and image
payloads. Pending decoded data is bounded; the chunk metadata index grows with
the number of published chunks. The writer avoids rewriting the growing
manifest for each chunk. Recording errors are surfaced through the run's
existing progress/publication failure path.

## Static export validation

The ordinary static exporter verifies complete replay channels before copying
their exact compressed bytes. It checks canonical chunk filenames, path
confinement, SHA-256, compressed sizes, bounded decompression, no trailing gzip
data, event counts, contiguous sequences and monotonic timestamp bounds. The
manifest totals must match its events. Limits are fixed by this version of the
reader. A malformed channel fails export rather than being silently omitted.

Missing older history and explicitly incomplete recordings export an unavailable
reason. Partial-channel recovery is still open. Exported `run.json` records the
available channel's relative manifest path. Publication review scans decoded
gzip checkpoint JSON within the same bounds, so compressed error text remains
subject to local-path and credential checks. Original recordings and source
run metadata stay unchanged.

## Static browser playback

The static run page offers an explicitly loaded, initially paused replay panel.
It supports play/pause, restart, ten-second jumps, a seek slider and speeds from
0.1 to 1,000 times real time. A monotonic virtual clock preserves position across
speed changes. The historical panel labels each checkpoint's own timestamp and
leaves the final workbook-result list unchanged.

The browser validates the manifest and chunk identities, limits compressed reads
and decompression, and retains at most two decoded chunks. It selects the last
recorded checkpoint at or before the requested time without interpolation.
Requests are same-origin relative HTTP/HTTPS fetches. File-URL playback is
unavailable, while the rest of the standalone report retains its existing
file-URL behavior. Static servers must serve `.json.gz` as raw gzip assets so
the browser can verify the retained compressed-byte identity.

Explicit seeks cancel obsolete requests. Paused playback schedules no animation
frames. During playback checkpoint refresh is limited to ten per second and
unchanged checkpoints do not rewrite the historical status. Page departure
cancels outstanding work. Missing or unsupported browser history shows an
explicit unavailable message.

## Browser acceptance

A bounded synthetic static export was exercised in the in-app browser and
Microsoft Edge on 2026-11-08. Its 128 checkpoints span four compressed chunks
and contain no scientific workbook comparisons. Loading, seeking to the final
checkpoint, restarting, 1,000-times playback, pausing, changing speed while
paused and a ten-second forward jump were observed. The source manifest and
export retained the same SHA-256. The owned loopback test server was stopped.

The report's content security policy permits same-origin connections for replay
assets. Fetch receives a root-relative path and rejects redirects. The browser
test exposed the missing connection policy that JavaScript-only tests could not
exercise. The private acceptance receipt is retained under
`G:\deltasieve-work\replay-static-acceptance-20261108\browser-acceptance.json`.

## Remaining delivery gates

A [100,000-checkpoint recording experiment](../visual-validation/experiments/replay-cost-20261108.md)
verified all retained event identities and bounded chunk sizes. It measured
17.369 seconds of recording wall time and approximately 3.30 MB of logical
history files in one debug-build trial alongside Enron. Process peak memory,
allocation overhead and broader long-run costs remain unmeasured.
Shared live process-component playback remains open.
Process/sheet/capture/heartbeat recording and long-run memory/cost acceptance
remain open too. This storage channel alone does not complete roadmap Task 15.
