# Resident capture events and timings

Resident comparison uses a persistent WebSocket control connection. PNG uploads
remain bounded HTTP requests. Physical clients advance independently through a
bounded window of common capture batches. Capture geometry, serial Office calls
within each workbook, exact pixel comparison and evidence retention remain intact.

## Delivery and recovery

The coordinator still publishes complete commands atomically. The host watches
command and control-file changes through native filesystem notifications. It
notifies connected clients when work or admission conditions change, and sends
the next job over the WebSocket when the client is ready to receive it. Idle
Office.js workers no longer request `/job` every 200 ms.

The coordinator subscribes to host changes instead of checking image receipts
and results every 100 ms. Events are bounded, coalesced wakeups: command files,
image receipts and result files remain the authoritative record. Consumers
reconcile those records after connection or reconnection, so an event arriving
during a disconnect does not erase completed work. Process checks and deadlines
remain separate from normal job delivery.

The first WebSocket message authenticates the connection. Worker tokens remain
scoped to their taskpane session, and coordinator access uses a separate token.
Tokens do not appear in WebSocket URLs. A retired session cannot submit work to
its replacement.

If the same pane reconnects while executing a job, its current execution is
retained. Receiving that job again must not execute it twice. If result delivery
loses its acknowledgment, the worker retries the same serialized result; the
host acknowledges an identical result from the same session and rejects a
conflicting one. A new taskpane document is a different session, not a resumable
socket connection.

Pause prevents dispatch of subsequent work. Already dispatched work may finish,
and uploads, result acknowledgments and control events remain usable while
paused. Heartbeats still report liveness independently of successful work.

## Bounded capture pipeline

The coordinator admits the current common batch and at most one following batch.
Each physical client still executes one command at a time. When both batches fit
in the same sheet and memory budget, one command contains two `capture_batches`:
exclusive tile end offsets into its existing `tiles` array. For example, `[4, 8]`
captures and finalizes four tiles, starts uploading them, then captures the next
four while those uploads finish. Office calls and overflow probes never overlap
within one client; there is one uploader per client.

A dedicated image-processing thread decodes PNGs, compares peers and publishes
previews. Its completion events wake the coordinator immediately. Credits are
returned only after comparison, so uploaded images and queued decoding count
against the same window as captures still running in Excel. A fast client can
advance without waiting for another client's whole command, but cannot build an
unbounded backlog behind a slow peer or slow image processing.

Each client's outstanding work is limited to 16 tiles and 64 million capture
pixels across commands. The Office.js worker additionally accounts for retained
base64 strings, binary PNGs and decoding buffers against a 160 MiB encoded-memory
budget. Office returns the initial base64 allocation before its encoded length
is known; this limit does not cover Office/WebView internals or all native bitmap
scratch space. The pixel limit bounds those capture requests separately. The
coordinator also limits unclassified PNG bytes to 160 MiB per client.

An equal PNG is removed only after both its comparison and its durable command
response have completed: the host validates that response against the PNG.
Differences and row-height evidence retain their full-resolution images. Matched
whitespace controls use the same pipeline for the already selected ranges.
Cancellation joins image processing before group cleanup can remove evidence.

For a paired reference experiment, set `EXCEL_SHRINK_CAPTURE_PIPELINE=0` before
starting the runner. This uses the previous synchronized coordinator and the
sequential client path. Unset it for the default pipeline. It is an experiment
switch, not a different capture or comparison policy.

## Preview reuse and equal images

Incoming images still appear immediately as previews up to 1024 pixels wide.
When the tile is classified, a matching preview is reused if its source pixel
identity, current-attempt URL and required dimensions match and the file exists.
Missing or incompatible previews are regenerated from the full-resolution pixels.
Changing the retained-preview width therefore still takes effect at classification.

Exact RGBA equality remains the comparison criterion. An identical peer needs no
difference mask; the original's combined mask is allocated only when a differing
peer requires it. Different images retain their full-resolution evidence, and
equal images needed for row-height reports retain their original evidence too.
These changes affect derived display assets, not capture coverage or whitespace
attribution.

## Measurements

The worker records elapsed durations with `performance.now()`. Rust records its
own intervals with monotonic clocks. These are local durations: timestamps from
different processes or clocks are not subtracted to manufacture network latency.

The worker detail panel shows the most recently measured command, which may
differ from its current job:

| Field | Scope |
| --- | --- |
| Execution | The complete worker command execution |
| Excel image capture | `Range.getImage()` Office round trips |
| Image finalization | Overflow validation, cropping and encoding/decoding needed by the existing path |
| Image upload / receipt | PNG request through its receipt acknowledgment |
| Waiting for job | Idle time including pauses and peer/startup readiness |
| Result delivery / acknowledgment | Sending the result through confirmed acceptance, including retries |
| Result delivery attempts | Actual attempts to deliver that command result |

Capture, finalization and upload are included in execution time; adding all
displayed durations would count the same time more than once. Other Office calls
may also contribute to execution. A missing measurement stays "Not measured";
an observed zero remains zero. The result acknowledgment becomes available only
after result acceptance, so it appears in a subsequent pane resource snapshot.

Worker samples contain only the last command's allowlisted fields, not an
ever-growing timing history. Timing samples do not advance the last-progress
timestamp or create successful-work events.

The group state also records coordinator wait, image decoding, preview and
comparison intervals and host upload intervals. Aggregates across independent
workers can overlap and must not be interpreted as sequential wall time.
Pipeline throughput uses elapsed capture-phase wall time, excluding intentional
pause and Excel-opening pauses; overlapping batch durations are not added. Group
state records the window limits and peak decoded tile/pixel counts per client.
Worker resources include peak accounted encoded bytes and the number of capture
chunks started while an upload was pending.

## Evaluation boundaries

Use identical capture plans, inputs, group counts and settings for comparisons.
Separate Excel startup and capture costs from transport costs; alternate test
order and repeat representative small, large, equal and differing cases. Count
fully completed comparisons, peak memory and failures alongside throughput.
An inconclusive short measurement is not evidence of a regression or speedup.

`office-capture/tests/pipeline-smoke.py --run` runs an isolated native Excel
baseline/pipeline/pipeline/baseline experiment. It checks identical tile coverage,
pixel hashes, differences and whitespace attribution before comparing timing.
The fixture contains both whitespace overflow and a chart referring to hidden
values. Samples use fresh Excel processes; OS and Office caches remain in place.
Receipts include startup, capture, whole comparison and peak Job Object memory.
Run the same script with `--run --pause-check --compare-to <completed-sample>`
for a separate untimed check: it pauses after initial captures, waits for active
commands to drain, resumes the same Excel processes and compares the final
evidence with the reference sample.
With `--baseline-binary <previous.exe> --binary <candidate.exe>`, the same harness
compares two builds with pipelining enabled for both. Set `--equal-preview-width`
to the same retained-preview width as the production run (currently 1024 on this
machine). It also checks decoded dashboard previews, full-resolution display
images and masks, independently of their filenames or encoding.

Local Windows check on 2026-10-26 (fixture v2, four primary Excel clients plus one
on-demand whitespace client, 1 MP tiles, batches of four, hidden Excel):

| Metric, median of two samples | Synchronized reference | Bounded pipeline |
| --- | ---: | ---: |
| Capture, upload and image processing | 6.633 s | 4.232 s |
| Whole workbook comparison | 31.170 s | 28.609 s |
| Runner wall time | 35.474 s | 32.741 s |
| Peak Job Object memory | 1.563 GB | 1.606 GB |

All four samples processed the same 41 tile positions and 167 physical images
(including controls), with identical pixel hashes, retained evidence, findings
and whitespace attribution. This supports the local pipeline improvement; two
samples per mode on one synthetic workbook do not establish corpus-wide speedup.
GB here means decimal gigabytes. The first functional warmup was excluded.

On 2026-10-27, a two-build ABBA check isolated preview reuse and equal-image mask
elision. Both builds used the bounded pipeline and 1024-pixel retained previews:

| Metric, median of two samples | Before preview reuse | With preview reuse |
| --- | ---: | ---: |
| Capture, upload and image processing | 4.150 s | 3.403 s |
| Whole workbook comparison | 28.582 s | 27.174 s |
| Final tile comparison and display publication | 1.554 s | 0.293 s |
| Initial preview publication | 1.228 s | 1.635 s |
| Peak Job Object memory | 1.591 GB | 1.621 GB |

The initial preview interval increased in this small sample; the savings came
from final tile processing. Those worker intervals overlap the capture phase and
are not additional wall time. All four runs produced identical scientific
evidence and decoded dashboard previews, full images and masks. A separate
512-pixel preview test paused after 32 images, drained to 64, then resumed the same
four Excel processes to 167 images with identical results. These are local checks,
not estimates of corpus-wide performance.

Optional hash-first transport, preview compression and WASM are separate
experiments and remain disabled.
