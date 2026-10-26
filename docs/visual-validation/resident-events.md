# Resident capture events and timings

Resident comparison uses a persistent WebSocket control connection. PNG uploads
remain bounded HTTP requests. This stage changes delivery and observation only:
capture geometry, per-workbook Office execution, synchronized batch boundaries,
pixel comparison and evidence retention keep their existing rules.

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

## Evaluation boundaries

Use identical capture plans, inputs, group counts and settings for comparisons.
Separate Excel startup and capture costs from transport costs; alternate test
order and repeat representative small, large, equal and differing cases. Count
fully completed comparisons, peak memory and failures alongside throughput.
An inconclusive short measurement is not evidence of a regression or speedup.

Capture/upload overlap, independent batch lookahead, optional hash-first
transport, preview compression and WASM are separate experiments. They are not
enabled by this event-delivery change.
