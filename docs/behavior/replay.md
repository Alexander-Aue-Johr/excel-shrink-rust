# Recorded execution history

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

## Remaining delivery gates

Browser readers, virtual-clock UI,
play/pause/seek/speed controls and shared live component playback remain open.
Process/sheet/capture/heartbeat recording and long-run memory/cost acceptance
remain open too. This storage channel alone does not complete roadmap Task 15.
