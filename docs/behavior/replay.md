# Recorded execution history

## SheetShrink Observatory

The native Observatory records workbook, worksheet and chunk events beside the
output directory in `<output>.events.jsonl`. After a run stops, **Replay recorded
run** reads that log in cursor-based pages. Playback supports pause, restart,
seeking and speeds from 0.05× to 2×. It reconstructs state from events observed
by the selected time, including metadata and completion reports. Playback does
not invoke cleanup or write workbook outputs. The displayed clock remains the
recorded elapsed time.

Observed SheetShrink runs also write `analysis.csv` inside their new output
directory, alongside `preparation.json`. It records per-sheet/per-cause cleanup
statistics under the normal cleanup policy. Statistics collection does not enable
the separate tail-pruning-disabled attribution policy. Unchanged inputs and
unsuccessful preparation outcomes must remain represented in workbook/status
tables when deriving corpus distributions.

Replay currently belongs to the current native server's run. Browser reloads
retain access while that server remains open. Reopening an older log in a new
server is not yet supported. A missing or malformed log produces an error.
The browser loads the full event history for replay, so its memory use grows
with the recording. Ordinary live polling retains its bounded history.

Completed distribution runs can also be exported through
`tools/export-preparation-replay.py` and opened over static HTTP, using the same
Observatory presentation. Export verifies contiguous sequences, monotonic clocks,
terminal events and preparation outcome counts. The manifest and compressed event
stream use relative same-origin URLs with a verified size and SHA-256. Playback
supports the recorded timeline, speed selection and backward seeking without a
live preparation server. `--redact-local-paths` projects standalone Windows paths
to basenames while retaining source and export checksums. Publication still
requires review of corpus labels and diagnostic prose. Full-history browser
memory remains an explicit limitation of this preparation replay path.

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

A fresh debug-process test on 2026-11-11 recorded and verified 100,000 synthetic
checkpoints in 3,125 chunks. Windows lifetime counters reported a 14.88-MiB peak
working set and 9.81-MiB peak committed memory for the entire test process.
The experiment and its scope are recorded in
`../visual-validation/experiments/replay-cost-20261108.md` and the adjacent
`replay-cost-20261111.json`. Detailed recording channels and browser memory need
separate measurements.

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

The preparation Observatory reuses its observed replay state during forward
playback. Each tick applies only events newly reached by the virtual clock.
Seeking backward discards that accumulated state and rebuilds through the
requested timestamp. Repeated timestamps retain the existing job/task snapshots,
and later seeks do not mutate previously returned snapshots. All recorded jobs
and tasks remain available for Atlas pagination and inspection. This removes
repeated event-prefix reconstruction while leaving full-trace loading, retained
history size and browser scene rendering as separate performance costs.

A bounded browser probe on 2026-11-11 loaded all 136,630 recorded Destatis
events through a read-only loopback proxy serving the current UI assets against
the completed run. Playback advanced, paused, and an End-key seek reached the
recorded 119.80-second endpoint. The final accessibility observation exposed
33,059 worksheet-selector items. The seek plus observation took approximately
25 seconds in the automation call, which does not isolate rendering time from
accessibility extraction. This exposes a remaining large-history UI cost and
does not establish responsive full-corpus playback. The temporary tab and proxy
were closed after the probe. Production native workers remained active.

Worksheet linking now renders pages of at most 100 matching worksheets, plus
the pinned selection when it falls outside that page. Search matches workbook
names, workbook identities and worksheet names without case sensitivity. Page
navigation preserves access to the complete observed worksheet history. A
backward replay seek clamps pages to the remaining observed matches and cannot
expose future worksheet metadata through the selector.

The follow-up browser probe exercised the paginated selector against the same
complete Destatis recording. The endpoint displayed 1,182 completed workbooks,
66,541 work items and 33,058 worksheets across 331 selector pages. Searching
`wa-gegenueberstellung` returned five worksheets across the matching workbooks.
Pause, End-key seek and the final observation completed in about seven seconds
in one automation call. The earlier call took about 25 seconds. These single
observations include automation and accessibility extraction, so they do not
establish isolated rendering speed or long-run memory behavior. The temporary
tab and read-only proxy were closed after both tests.

A [100,000-checkpoint recording experiment](../visual-validation/experiments/replay-cost-20261108.md)
verified all retained event identities and bounded chunk sizes. It measured
17.369 seconds of recording wall time and approximately 3.30 MB of logical
history files in one debug-build trial alongside Enron. The subsequent fresh
test process peaked at 14.88 MiB working set and 9.81 MiB committed memory.

The production JavaScript reader also traversed the complete 100,000-event
recording through a loopback static HTTP server and completed 128 nonsequential
seeks. It retained at most two decoded chunks containing 64 events. Twelve
explicit post-GC samples ranged from 9.10 to 11.53 decimal MB of used JS heap
and 79.27 to 108.55 MB RSS for the combined Node server/test/reader process.
These samples exclude transient allocation peaks and browser UI costs.
The reproducible command and receipt are in
`../visual-validation/experiments/replay-cost-20261108.md`.
Allocator attribution, browser long-run memory and broader recording costs
still require separate measurements.
Shared live process-component playback remains open.

## Standalone SheetShrink replay export

`tools/export-preparation-replay.py RUN_DIRECTORY NEW_DESTINATION` exports a
completed distribution recording with the shared Observatory assets. Serve the
result through HTTP or HTTPS, including from a GitHub Pages subdirectory. The
export uses relative URLs and requires no running preparation API. Existing
export directories are rejected. The original compressed event stream is copied
byte-for-byte, checked against terminal outcomes, and recorded with its SHA-256.
The browser verifies the compressed payload before decoding it.

Playback supports pause, backward and forward seeks, and speeds from 0.05 to
1,000 times. Worksheet search retains paginated access to the complete history.
The jobs table displays 256 rows per page. The complete Enron browser probe
loaded 30,301 workbooks, 120,741 worksheets and 241,627 work items. Job pagination,
rewinding to zero and returning to the end succeeded without console errors.
That probe exposed and corrected a renderer argument-limit failure when adding
a large staircase geometry to its vertex batch.

This exporter loads the entire recording into browser memory. Its input limits
are one million events, 128 MiB compressed and 512 MiB decoded. These limits do
not establish bounded JavaScript heap use or long-run playback performance.
The exported publication status remains `review_required`. Workbook labels,
error messages and dataset redistribution rights require review before public
hosting. These preparation recordings supplement the validation report and its
separate chunked replay format.
Process/sheet/capture/heartbeat recording and long-run memory/cost acceptance
remain open too. This storage channel alone does not complete roadmap Task 15.
