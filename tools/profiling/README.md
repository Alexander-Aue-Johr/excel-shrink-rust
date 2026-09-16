# Profiling Excel Shrink

## 1. Record the parallel pipeline

In VS Code, choose **Terminal > Run Task > Profile smoke test (pick case, Perfetto, no debugger)**.
The task builds with release optimizations and debug symbols, then records one smoke case.
It prints the unique trace filename under `.tmp/profiles/`. No debugger or extra profiler
installation is needed for the timeline.

Equivalent command:

```bash
cargo run --profile profiling -p excel-shrink-cli -- regression smoke \
  --case-id overall-reduction --trace-file '.tmp/profiles/smoke-{timestamp}.json'
```

To profile ordinary cleanup using the default cleanup policy:

```bash
cargo run --profile profiling -p excel-shrink-cli -- input.xlsx .tmp/profile-output \
  --force-overwrite --trace-file '.tmp/profiles/cleanup-{timestamp}.json'
```

Trace paths follow `--working-directory` for ordinary cleanup, and the launch working
directory for the regression command. `{timestamp}` expands to a unique time/PID suffix.
Existing files are never overwritten. Tracing is disabled unless `--trace-file` is passed.
The trace output is separate from `--console-log-level`, `--file-log-level`, and analysis CSV.

Parallel ranges default to **16 MiB** of uncompressed XML (`--parallel-range-chunk-size
16777216`), split at row boundaries. A single oversized row stays intact. This limit is
separate from `--chunk-size`, the streaming read buffer. Use `--parallel-range-chunk-size
134217728` to compare the previous 128 MiB limit, or `0` to disable range chunking.
For a regression run, forward these as `--cli-arg=--parallel-range-chunk-size
--cli-arg=134217728`.

Open [Perfetto](https://ui.perfetto.dev), select **Open trace file**, and load the JSON.
You can also print a summary without additional Python packages:

```bash
python3 tools/profiling/summarize.py .tmp/profiles/<recording>.json
```

### What to look at

- `initial_sheets_fork_join`: the complete first parallel phase, including nested work.
- `sheet_initial_cleanup` and `clean_range_chunk`: individual leaf/parent tasks. The
  last completing chunk or sheet may explain why the next phase cannot start.
- `validate_references` / `scan_sheet_references`: the serial reference phase.
- `sheet_finalize`: deferred cleanup, collecting remaining cell references, compression,
  and queuing output. This excludes the preceding wait for other sheets.
- `record_stats_and_queue_sheet` / `publish_cell_references`: handing off results,
  including acquiring the lock on the workbook's cell-reference map.
- `release_sheet_metrics`: freeing the retained cell bookkeeping and sheet summaries after queuing a sheet.
- `wait_workbook_permit`: blocking because the active workbook limit was reached.
- `writer_wait_for_job`: a receive operation, including any time blocked on the channel.
- `writer_copy_entry` / `finish_zip`: copying prepared/original entries and finishing output.
- `wait_analysis_writer`, `wait_writer_pool`, `wait_watchdog`: joins at batch completion.
- Handoff arrows: scheduling to worker start, initial result to finalization, and
  producer to writer. Select the endpoint slices (`flow_start` / `flow_end`). Their
  arguments include workbook/sheet identity and the shared flow ID.
- `writer_pending_jobs`, `writer_pending_compressed_bytes`: queued output waiting to be
  taken by a writer. Byte counts include precompressed buffers only, not raw-copy source
  entries or the entry currently being written.
- `initial_results_buffered_bytes`: initially cleaned XML retained until finalization
  begins. This is not total RSS or all buffers alive inside the cleaner.
- `xml_rebuild`, `cleanup_candidates`, `changed_rows`: work volume at deferred passes.

### Inside streaming preprocessing

`preprocess_sheet_data_node` includes stream reads while in the sheet-data state.
Its new child phases separate:

- `preprocess_read_decompress`: `Read::read`, including ZIP decompression for XLSX input; not pure decompressor CPU time. Its `requested_bytes` and `bytes_read` arguments show buffer size and actual uncompressed bytes returned (zero means EOF; a failed read can leave `bytes_read` absent).
- `preprocess_append_input`: copying newly read bytes into the input buffer, including growth.
- `preprocess_find_sheet_data_end` / `preprocess_find_row_boundary`: locating a processable prefix.
- `preprocess_sheet_data_prefix`: inclusive parent of preprocessing a prefix.
- `preprocess_check_cdata`: unsupported CDATA scan.
- `preprocess_check_style_risk`: row/column style safety checks, only when early empty-cell cleanup is enabled.
- `preprocess_remove_empty_cells`: cell scanning and optional rewritten output construction; absent when style risk skips cleanup.
- `preprocess_append_result`: appending the processed prefix to retained sheet XML, including allocation/growth.
- `preprocess_retain_buffer_suffix`: copying the unconsumed suffix and releasing the old input buffer.

Read/input/suffix phases can also occur under other streaming states. These are
per-buffer phases, not per-cell events. They measure inclusive wall time; do not
add `preprocess_sheet_data_prefix` to its children. They do not separate allocator
CPU time from copying, or cell scanning from rewritten-buffer creation.

### Before and after explicit drops

Explicit drops in the cleanup pipeline and row/chunk processing emit `before_drop`
and `after_drop` markers. Match them by `drop_id`; `resource` identifies the value,
and `source_file` / `source_line` locate the call. The first marker also gives its
Rust type. The second contains `elapsed_ns`, measured around destruction and excluding
the emission of the two endpoint markers.

The `sheet.metrics` drop additionally exposes all 15 fields in declaration order,
with individual `drop.sheet.metrics.*` slices and marker pairs. The old `row_attributes`, `kept_row_states`,
`deleted_row_states`, and `rows_with_kept_content_cells` collections no longer exist;
new traces therefore have no drops for them. Scalar fields have no heap storage
to release. The aggregate measurement includes field instrumentation and must not be
added to its children.

Resources ending in `_span` are profiling guards: their destruction closes a measured
scope. Closing such a guard can change the parent span between the before/after markers,
so use `drop_id` to pair them. These markers use no enclosing measurement span themselves,
which avoids overlapping scope boundaries. Detail guards are still recorded only for
sampled rows; absent optional guards generate no markers. Recording is disabled without
`--trace-file`. Implicit Rust drops elsewhere are not intercepted globally. If destruction
panics or the process is terminated, a before marker may have no after marker.

### Inside `clean_cells_range`

With tracing enabled, the first row and then roughly one in 4096 rows **of each
range/chunk** gets a `sampled_row` slice. Deterministic jitter varies the interval
between 2048 and 6143 rows to avoid repeatedly sampling power-of-two buffer growth.
The slice includes the actual XML row reference. Zoom into it to inspect:

- `row_find_block`: locating the row and its XML boundaries.
- `row_inspect_header`: one forward attribute scan into a borrowed `RowView`, without
  allocating attribute strings or cloning styles.
- `row_record_summary`: updating visible bounds/tail anchors when the row style requires it.
- `row_scan_and_clean_cells`: scanning and cleaning the cells in the sampled row.
- `row_prune_style_representatives`: removing provisional representatives when unnecessary.
- `row_render_xml`: rendering the row, including updating its attributes/spans.
  Its children show `row_rewrite_spans`,
  `row_clean_empty_attributes`, and `row_build_xml` where applicable.
- `row_copy_prefix` / `row_append_output`: appending to the range's output buffer.

Select the `sampled_cell_work` event inside `row_scan_and_clean_cells` for cumulative
nanoseconds spent locating cells, parsing their XML/attributes/content, selecting style
representatives, applying cell cleanup policy, recording representatives, and appending
cell output. These timers accumulate locally; there is no trace event per cell.
`range_row_samples` reports total and sampled row counts and input/output bytes.
Chunk setup/assembly has separate `clone_base_metrics`, `clone_chunk_metrics`,
`append_chunk_xml`, and `merge_chunk_metrics` slices.

The row detail is a **sample, not a complete breakdown**. Short chunks are
sampled more densely; rare row patterns can be missed. Timer and tracing costs
affect very short measurements. Do not multiply sampled times by the sampling interval or subtract them
from the parent duration to infer another stage's cost. With tracing disabled the normal
row/cell specialization performs no per-cell clock reads or detailed trace emission.

`sheet_timing` separates `initial_cleanup_ms`, `wait_before_finalize_ms`, and `finalize_ms`.
The middle value includes waiting for the remaining sheets, the reference scan, and
scheduling before this sheet's finalization. It is not OS thread sleep time.

**All spans are inclusive wall times, not CPU times.** Rayon workers can execute another
job while a parent parallel operation has not returned. Nested/concurrent durations
must not be added to obtain runtime. Parent span IDs describe instrumented execution
scope; explicit handoff IDs describe work transfers across threads. Thread IDs in this
portable JSON exporter are recording-local identifiers, not operating-system TIDs.

The bounded recorder queue never blocks worker threads. If it overflows, stderr warns
and `otherData.dropped_events` is nonzero: that trace is incomplete and should not be
used for precise dependency analysis. Trace serialization runs in its own thread but
still consumes CPU and memory; compare timings with a tracing-disabled run. A normal
return, including a reported CLI error, flushes the trace; forced process termination
can leave an unfinished JSON file.
Canceled or failed work can leave a handoff without a receiving endpoint.

## 2. Find CPU hotspots

Build once, then profile the executable rather than Cargo itself:

```bash
cargo build --profile profiling -p excel-shrink-cli
cargo install --locked samply
samply record ./target/profiling/excel-shrink-cli regression smoke --case-id overall-reduction
```

If `CARGO_TARGET_DIR` is customized, use the corresponding executable location.
On Windows the executable has an `.exe` suffix. See the [samply documentation](https://github.com/mstange/samply)
for installation and OS permissions. Linux/WSL sampling requires usable perf events;
samply currently collects on-CPU samples on Linux, not complete off-CPU stacks.
If it reports `perf_event_paranoid=2`, the system's sampling permissions must be adjusted
before samply can record as a normal user; this repository does not change kernel settings.

Start with the phase that delays the overall pipeline, then inspect its call stacks
for XML scans, allocation/copying, reference lookups, or compression. Debug symbols
preserve release optimization; they do not start a debugger.

For scheduling questions that application spans cannot answer, use Linux `perf sched`
or system scheduling data in Perfetto if the kernel and permissions support it.
For allocation churn use [heaptrack](https://github.com/KDE/heaptrack) in a separate run.
Neither instrumentation nor CPU sampling proves why an algorithm performs unnecessary
work: use the counters, source code, and a controlled change to test that hypothesis.
For example, compare `changed_rows` with the `xml_rebuild` byte counts: repeated large
scans after very few changes are candidates for skipping a pass or limiting it to
affected rows. Verify the reference and layout dependencies before changing those passes.

## 3. Confirm improvements

Use the same workload/options with tracing disabled for final timing comparisons.
Keep compilation, downloads, output validation, Excel and screenshot work outside the
cleaning measurement (`shrink_duration_ms` in regression reports). A smoke case can
also enable analysis or aggressive cleanup options; it is not necessarily the default
CLI policy. Use the existing `tools/corpus-compare` comparison/benchmark runner to
compare output and repeated before/after runs. Report the median and variation; repeat
with both a large sheet and a batch of workbooks before generalizing a speedup.

## Row processing and allocation contracts

`sheet::row_view::RowView` borrows the current XML header and workbook style. The
row orchestrator inspects the header, records required summaries, cleans cells,
and renders the result; byte scanning is isolated in `xml::attributes`. Both row
and cell headers are scanned forward once, reusing the reference attribute's value
and assignment. Quoted values, reordered attributes and whitespace around `=` are
supported; malformed headers fall back to unchanged XML.

Only the current row retains its content flag, maximum kept column and temporary
style representatives. The common spans-rewrite path uses that maximum; after
pruning provisional representatives, only the current row's cell records are
rescanned. Older cell records are not removed by that pruning. Recleaning computes
these values anew rather than consulting stale state from the previous pass.
Already self-closing, unchanged rows return borrowed XML while still updating any
required style bounds. Output/range buffers, changed XML, cell records, reference
candidates and necessary bounds can still allocate: the whole cleanup is not an
allocation-free operation.

Chunk setup copies only shared input metadata; chunk merging carries no row
history. `tests/row_allocations.rs` uses a thread-local counting allocator to verify
zero allocations for repeated header inspection and both unchanged-empty-row paths.
The public core API replaces owned `RowAttributes` and the four row collections in
`CleanSheetMetrics` with `RowView`; `cells::replace_cell` now accepts an optional
borrowed row view for callers that clean individual regex matches.

### Further allocation reductions

Attribute lookups in the bounds/range and output passes use static complete byte
patterns, as does formula detection. `deleted_cell_count` replaces the unused
per-deleted-cell history. The chunk planner consumes row ends directly and retains
only chunk boundaries. Preprocessing stores numeric relevant-style IDs and no
longer keeps the ineffective checked-ID cache; canonical byte-key behavior is
preserved for zero-padded or invalid IDs. Additional allocation tests cover the
visible-bounds scan and formula detection with reusable metrics.

Temporary style representatives now borrow their reference attribute from the
stable input row. Already self-closing retained cells return borrowed XML as well,
which avoids allocating a cell buffer and triggering an otherwise unnecessary row
rewrite. Changed rows still use output buffers; this is not a full direct-writer
redesign. Allocation regressions measure bounds scanning and formula detection
separately from necessary output/metadata allocations.

### Conditional original-data bounds

The full original visible-data scan runs only for `KeepInDataRange`, its only
consumer. Other policies leave `visible_data_range_bounds` empty; actual visible
content/tail bounds continue to be collected during cleanup. Existing protection
and later-visible-content regression tests cover the policy behavior.

An optimized, untraced `overall-reduction` comparison (ABBA then BAAB, four runs
per binary) measured median cleanup time 5.867 s before and 4.595 s after (21.7%
lower). All 225 unpacked ZIP entries match across all eight runs. Workspace tests:
240 passed, six existing ignored, full corpus suite filtered. Raw reports and
binary hashes: `.tmp/profiles/optimization-series/step1/results.json`.

### Reuse fixed XML token searchers

`xml::find_bytes` retains immutable `memmem::Finder` instances for the eight common
cell, row, value, formula and inline-string tokens. Each finder is initialized once
per process through `OnceLock` and shared by workers. Other byte patterns retain
the general search path. This does not change substring/tag-boundary semantics.
A regression compares all substrings of a mixed XML/byte fixture against the
original search, including truncated tokens, empty and uncached patterns.

After CPU sampling identified repeated Finder preparation, an optimized untraced
ABBA/BAAB comparison on `overall-reduction` measured 4.893 s before versus 4.2235 s
after (13.7% lower median, four runs each). Every changed run was faster than every
baseline run; both four-run blocks improved. All 225 unpacked ZIP entries match
across all eight outputs. 241 workspace tests passed, six existing tests ignored,
full corpus suite filtered. This is evidence for the tested workbook/profile,
not a guarantee for every input. No additional smoke corpus inputs were available
locally. Reports, saved binaries and hashes: `.tmp/profiles/finder-reuse/`.

### Combine cell-boundary and content inspection

Row cleanup now detects value/formula/inline-string markers while locating the
cell block's end. Self-closing cells need no separate content search. Once a
content marker is found, the remaining scan only locates `</c>`. Literal '<' bytes
in malformed headers retain the old substring-based fallback. The regex-based
individual-cell entry point retains its existing content check. Sampled cell
profiling now attributes content detection to `ParseBlock`; comparisons with old
`ParseAttributesAndContent` timings must account for this move.

A manual optimized benchmark over 65,711 real cells from four worksheets measured
median 241.777 ms before and 187.099 ms after for 100 repetitions (22.6% lower).
Run the ignored test `benchmark_fused_cell_scan` with `CELL_SCAN_FIXTURE` pointing
to XML containing complete `<c ...>` blocks. It has no timing assertion.

An untraced optimized ABBA/BAAB full-workbook comparison measured 4.120 s before
and 3.581 s after (13.1% lower median, four runs per binary). Both blocks improved;
all 225 unpacked ZIP entries match across eight outputs. 242 workspace tests pass;
six pre-existing tests and the manual benchmark are ignored in normal runs, and
the full corpus suite is filtered. The manual benchmark was run separately.
Results cover one workbook/profile and are not a general speed guarantee.
Artifacts and binary hashes: `.tmp/profiles/fused-cell-scan/`.

### Reuse column context per cell

The row cleaner and regex cell entry point resolve the column definition once,
then pass a borrowed view of a local scalar-only snapshot through cell policy
checks. Copying ColumnDefinition/Xf here allocates no heap storage and permits
mutation of cleanup metrics. Cached absence is distinguished from an unresolved
lookup; test/helper callers without context retain the original lookup fallback.

A full nine-case smoke comparison (eight workbooks, ABBA/BAAB, four suite runs per
binary, no debugger/trace) measured median summed cleanup time 9.737 s before and
9.003 s after (7.5% lower). Every case's median improved; outputs match per ZIP entry
across 72 case runs. 242 workspace tests passed with the corpus wrapper filtered;
the corpus cases were executed by the benchmark runner. Measurements are specific
to these inputs and host. Artifacts: `.tmp/profiles/next-three/columns/`.

### Visible sheet labels

Sheet-owned spans use labels such as `[sheet84] clean_range_chunk`, including child phases inherited from that sheet. The number comes from the ZIP part `xl/worksheets/sheet84.xml`, not the workbook tab order. `args.sheet` and `args.xlsx` retain full identity; `args.chunk` retains the chunk index. `args.phase_name` preserves the original phase for grouping across sheets. The summary script supports both old and prefixed traces. Across workbooks, use `args.xlsx` to distinguish identical sheet labels.

### Reusable reference scanner

`PreparedReferenceScanner::new` prepares the complete sheet-name mapping and regex references once. Its `scan(&self, ...)` borrows that immutable context; concurrent callers use separate `ProtectedWorkbookRefs` outputs. Include every sheet name when preparing, even if later scans only examine a subset. The compatibility function still prepares per call. The CLI prepares once before initial sheet processing. Sheets without deferred candidates publish their reference information and finalize early. Early scans currently serialize updates to the shared protection map; immutable scanner lookup data needs no lock itself. Remaining sheets finalize after the aggregate reference check.

### Byte lookup experiment

Prepared scanner sheet-name keys and retained calcChain cell-reference keys use owned byte vectors, with borrowed byte lookups. Only escaped sheet names allocate a temporary lookup buffer. Reference storage still allocates per distinct address. Checked unsigned ASCII parsing avoids UTF-8 intermediates in numeric cleanup fields; malformed-header UTF-8 fallback is retained. Final untraced nine-case comparison: 6919.5 → 6916.5 ms median, no demonstrated overall speedup; all 72 outputs match per ZIP entry. Artifacts: `.tmp/profiles/byte-lookups/final/`.
