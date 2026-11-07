# Execution, results and evidence contract

These rules complement [the shrinker](shrinker.md) and
[the targeted comparison](targeted-validation.md). They distinguish scientific
outcomes, scheduling, diagnostic telemetry and disposable working files.

## E1. Corpus and workbook identities

Keep the original corpus/archive identity and catalog when processing a subset.
A workbook identity combines corpus ID, archive member index and member name.
Record filenames use a shortened hash of that full identity. A filename alone
is insufficient to identify a result, especially across corpora.

The catalog records uncompressed input size and ZIP identity information.
Partitions are deterministic size-ranked ranges of approximately equal **file
count**, not equal total bytes. Tie ordering uses names/indices. Processing
ascending or descending inside a partition does not change its membership.

`--workbook-list` is a JSON array of full identities: validate duplicates,
unknown identities and partition membership, then restrict/order only the pending
selection. Completed records must not be resurrected because they appear in this
list. Save the selection in the manifest. `--limit` is applied to new pending work,
not a license to truncate a workbook's capture coverage.

The source fingerprint, profile configuration, shrink producer identity and
selection provenance are recorded independently. A copy with the same display
name is not automatically the same input. Do not manufacture a different small
corpus ZIP merely to prioritize original workbooks if their results must resume
under the original identities.

## E2. Preparation versus comparison versus completion

Preparation extracts/converts the source if needed, runs required shrink profiles
and stores reusable ZIP bundles. Targeted mode requires Default only; Check
Performance is an Excel-side comparison state, not another shrink profile.
Preparation success does not mean visual validation has occurred.

A reusable preparation requires the ZIP to exist, matching source fingerprint,
accepted/current producer and valid evidence for every requested shrink profile.
An approved prior producer is an explicit compatibility decision, not automatic
permission to reuse every old executable's outputs after arbitrary shrinker changes.

The targeted completion predicate requires `complete=true`, a compatible recognized
capture policy, the current execution strategy and producer status
`completed_with_limitations`. Completed evidence including Check Performance also
satisfies a Default-only request; a request for Check Performance requires that
additional evidence. It is metadata-based; it does not read and recompare
all retained images on every resume. Fresh report publication now validates its
pair/region/verdict contract before classifying success. Auditing old artifacts
is a distinct operation, not something a completion flag proves retrospectively.

The scheduling repair in this review series uses the effective requested profile
for both initial filtering and the later missing-preparation check. A Default-only
targeted success must not be requeued because unused Maximum/Aggressive profiles
are absent.

Default CLI behavior retries skipped records once per invocation. Use
`--no-retry-skipped` when intentional exclusions and past failures should stay
terminal for that invocation. This flag prevents retries; it does **not** certify
those files. `--compare-only` prohibits a new shrink preparation. See the
[operator commands](../visual-validation/targeted-corpus.md).

Preparation dispositions such as namespace exclusion, no-change/package identity
and “needs visual” remain separate from successful comparison outcomes. A byte-
identical Default preparation does not bypass requested Check Performance work.

## E3. Admission, pause, cancellation and timeouts

`target_groups` controls concurrent workbook groups, not the number of Excel
processes. The current differing-input Default comparison uses two Excel instances
and rebuilds Default in place. Explicit Check Performance adds one instance.
Identical-input omissions can reduce that count. Historical six-state runs retain
their own provenance. Reducing the target affects admission; already admitted
groups are allowed to finish.
Target zero drains work and then pauses. It must not kill an in-flight capture
merely to meet the new target.

The control room has additional legacy capture-pause/tile settings. Their presence
in JSON does not prove that each targeted phase is dynamically interrupted by
them. Distinguish group admission/pause from pausing command dispatch inside a
workbook. In particular, changing an admission limit is not force-cancellation.

Operation deadlines, workbook deadlines and bounded close/cleanup waits are
different. The native host requires responses from commands; close is bounded by
the smaller of its operation timeout and 15 seconds. Cleanup helpers have their
own bounds. A timeout records a failed attempt and retains context, rather than
silently advancing as if the missing work finished.

Opening processes and Add-in readiness are separate observations. Delayed startup
under concurrent Excel load does not by itself establish a deadlock. Conversely,
PID liveness or an Add-in heartbeat alone does not prove command progress.

## E4. Windows ownership and concurrent registration

Start native `excel.exe /x` clients with the staged Office.js add-in. Use an owned
private desktop for the targeted workflow and separate session/WebView staging.
Do not resolve ownership from a process name such as “all EXCEL.EXE processes.”
Bind process observations and termination to PID **and creation time**, retaining
owned process/job handles where available. A recycled PID must not be killed.

`windows-owners.jsonl` is the launcher's **ownership journal**, not a workbook's
comparison result. Multiple independent launchers can register for one workbook.
The path-specific Windows-session mutex serializes only complete journal appends
and cleanup snapshots. Opening, capture and other groups remain parallel.

Registration checks the attempt generation and stopping marker while holding
that mutex, then checks again after append. Cleanup marks stopping before taking
its locked snapshot. A racing/expired launcher cannot proceed to spawn Excel as
if it belonged to the current live attempt. Lock acquisition is bounded, and an
abandoned mutex can be acquired; exceptions release ownership.

Cleanup must distinguish owned, already gone, reused and unverifiable identities.
Birth-time inspection requests only limited query access, not synchronization or
termination rights. Protected replacement processes may deny those stronger
rights while still allowing creation-time verification. Query exit status through
the identity handle; request wait/termination access only for a matching owner,
and recheck creation time through that second handle before terminating it.
An unverifiable potentially live owner prevents a false “all stopped” claim.
Confirmed-exit receipts cannot authorize terminating a different process birth.
Malformed ownership entries are not ignored as harmless empty results.

Only verified known temporary staging roots may be recursively removed; reject
reparse points before traversal. Once all owning processes are confirmed stopped,
locked leftover stage files are a storage-cleanup warning and need not block all
future workbook comparisons. Preserve deferred-cleanup evidence.

The former shared cleanup gate is not a requirement to keep every peer group
alive until the slowest finishes. Owned processes should close when that group's
evidence needs are satisfied. Within a targeted group, keeping unrebuilt states
for late candidate backfill is intentional and has a separate reason.

## E5. Check Performance and Windows/COM dependencies

Check Performance requires a **positive native outcome**, either confirmed
optimization or no optimization needed. Merely opening its pane is not success.
The native adapter operates on the group's existing Excel copy/private desktop.
It is distinct from the normal Office.js workbook command API. Excel build,
localization, UI availability and native adapter support can affect this phase.

Office.js handles ordinary save, calculation, cell captures and row-height work.
The current workbook calculation endpoint uses `CalculationType.fullRebuild`,
waits for calculation completion with an internal bound, then saves.
Legacy XLS -> XLSX conversion and standalone chart-sheet fallback still need
Windows Excel automation. The Rust servers do not eliminate those platform
dependencies. Extra COM-only timing workbook opens are not part of the specified
targeted startup measurement.

Startup time is launch -> verified Add-in readiness: it includes workbook opening,
task-pane initialization and first successful workbook/host communication. It is
not a pure `Workbooks.Open` benchmark. Read process memory externally against its
exact identity at that observation. Do not wait for a partner merely to timestamp
an already ready client, or present one readiness sample as a complete memory peak.

## E6. Authoritative result and failure semantics

| Artifact / field | Meaning |
| --- | --- |
| `records/<identity hash>.json` | Durable corpus record, provenance and selected result references |
| Prepared ZIP | Reusable raw/conversion/shrink inputs and preparation measurements |
| `report.json` | Completed targeted pair verdicts and scientific diagnostics |
| `partial-report.json` | Provisional/incomplete observations; cannot certify whole-attempt success |
| `failure.json` | Explicit failure reason for the attempt |
| `portable-report.json` | Report with transferable relative artifact references |
| `xml-changes/NN.jsonl.gz` | Detailed retained XML change evidence for a transition |
| Telemetry / queue | UI observations and scheduling; not a replacement for report evidence |

Classify only after the command succeeds, the completed report is readable and
its required pair/region structure is valid. A difference is the union of explicit
pixel, geometry, chart-presence and rendering flags across requested pairs.
Missing Boolean flags must not become `false`; missing arrays must not become
empty successful comparisons. The report contract also rejects duplicate pairs,
duplicate regions, mismatched region sets and missing image references.

An “unchanged” targeted result concerns its recorded scope. Namespace exclusions,
technical failures, interrupted attempts, zero-area chart observations and zero-
candidate reports are different categories; their counts must stay interpretable.

## E7. Evidence compression and deletion ordering

Write every retained XML change to gzip JSON Lines, with a schema header and
shared context dictionary. Rows retain sheet, location, before/after identities
and opening/saving overlap. The main report stores counts and a reference instead
of duplicating millions of change objects. These are fingerprints of XML content,
not necessarily literal full cell XML; the CF inventory is a separate source of
raw rule-node evidence. Exterior omissions are counted, not invented as gzip rows.

For schema 1, the gzip's first JSON line names `schema`, a `contexts` array and
the ordered `columns` array:

```text
sheet, location, before, after, context_before, context_after, also_in_original_open_save
```

Subsequent lines are arrays in that order. Context values in each row are integer
indexes into the header dictionary; before/after values can be null for absence.
`location` distinguishes `cell:A1`, row metadata and column/default changes.
The `.pending` gzip is finished, flushed and synchronized before being renamed
and referenced by the report. A pending file is not a completed audit.

Compare original-resolution pixels **before** creating equal-image previews.
Keep different regions and their eight captured grid neighbors at full resolution.
The neighborhood is among captured regions: it does not retroactively acquire
otherwise absent neighbor screenshots. Equal regions can use the configured
preview width; zero preserves full resolution. Encoded duplicate images can share
one retained file without collapsing distinct verdicts.

Persist original image hashes, original/retained dimensions, mappings and verdicts.
First publish the completed and portable reports referencing durable retained
files; only then delete superseded images. Charts keep their separate evidence.
Remove disposable XLSX states only after scientific evidence is published.

`workbook_bytes` records raw-input and working-state sizes before deletion,
including original/default before save, saved states, Check Performance before/
after optimization and rebuilt states. Missing this field in older reports means
**not recorded**, not zero bytes or zero cleanup. The old states generally cannot
be recreated bit-for-bit by opening/saving again, especially with volatile input.

For comparisons of reduction, distinguish:

1. raw input -> Shrink output;
2. raw input -> ordinary Excel-saved Original;
3. Check Performance's own saved baseline -> its optimized/saved result;
4. opening/saving/rebuilding of subsequent copies.

Do not attribute all bytes removed by an ordinary Excel save to Check Performance.

## E8. Transfer, merge and backups

Portable bundles carry records, referenced images/audits and machine/run manifests.
Import checks corpus/catalog compatibility, package paths and artifact hashes.
Repeated identical imports are idempotent; conflicting records are retained for
review rather than silently overwriting another machine's result. Artifact hash
integrity is not a new independent visual comparison or a proof of report semantics.

Do not transfer absolute source-machine paths as the only usable evidence links.
Use the portable relative references on the receiving machine. Preserve original
producer/machine attribution instead of crediting imported work to the receiver.

Historical image/result groups may be moved to another disk only after verified
copying; keep original report paths reachable, e.g. with checked directory
junctions. Both metadata and external evidence disks then form the backup.
Moving old immutable evidence does not authorize deleting current inputs,
prepared ZIPs or an active attempt. A fresh comparison reset must not reset
reviewed terminal exclusions merely because they have no prepared ZIP.

## E9. Telemetry and the Observatorium

- Cyan stationary chip/gear: Windows process liveness for exact PID/birth.
- Green Excel symbol: opening / known workbook lifecycle / confirmed closure.
- Violet XML symbol: timestamped host XML analysis activity.
- Heart curve: only actual Office.js heartbeat messages, never PID probe ticks.
- Capture progress: selected-region work; discovery counts can temporarily be
  raw candidate counts and must not be presented as measured differences.

The current read-only Observatory adapter joins active records to their exact
targeted run and resident metrics. It validates PID and creation time before
exposing received Add-in heartbeat timestamps/counts. Repeated API reads do not
generate new beats. Historical cards without such data must not substitute PID
heartbeats. The display is optional; closing it must not stop comparison work.

Heartbeat absence means no recent Add-in signal, not automatically an Excel
deadlock. A blocked Excel command, starting/reloading pane or CPU pressure may
explain delay. Keep heartbeat age, process liveness and actual work progress
separate. Glow/afterglow conveys recent activity, not a new measurement.

## Source map

| Rules | Main source under `excel-shrink-cli/` |
| --- | --- |
| E1-E3 | `src/regression/archive/study/{mod,selection,preparation,progress,scheduler}.rs` |
| E4 | `src/regression/archive/worker.rs`, `src/regression/archive_processes.ps1`, `ArchiveProcesses.cs`, `office-capture/TargetedResident.psm1` |
| E5 | `office-capture/CheckPerformance.psm1`, `TargetedChartSheets.psm1`, `resident.ps1`, `web/targeted-diagnosis.js` |
| E6-E7 | `src/regression/archive/study/prepared/targeted.rs`, `targeted/{contract,retention}.rs`, `src/regression/targeted_comparison/{reporting,change_evidence}.rs` |
| E8 | `src/regression/archive/study/{transfer,provenance}.rs`; local backup scripts are deployment artifacts, not core requirements |
| E9 | `src/capture_host/control_room/targeted.rs`, `office-capture/control-room/{targeted-model,process-icon-shader,targeted-panel}.js` |

### Adaptive capture evidence compatibility

`capture_partition: visible-geometry-v1` changes image partitioning, not the
selected coverage union or the `targeted-content-grid-v2` coverage policy. Earlier
completed grid results remain reusable with their original producer identity.
New reports require explicit side geometry, rendering states and `pixels_compared`;
a null image is legitimate only for verified zero-area geometry. Missing or
positive-area image evidence still fails the completed-report contract. Adaptive
retention preserves differing rectangles and their actual neighboring cell
rectangles at full resolution, including neighbors at a large rectangle's far
edge. Confirmed-equal other images can still become deduplicated previews.

### Version-three state selection and resume

`targeted-content-grid-v3` adds explicit byte-identity and conservative no-rebuild
exclusions. The actual pair set is derived from the six `state_decisions`, not
assumed to contain five/nine comparisons. Every pair whose endpoints are observed
remains required. Missing actual observations cannot be replaced by empty arrays.
Independent native Check Performance confirmation remains mandatory when requested.

Complete version-two results are still accepted when their Check Performance
inclusion matches the request: they used the earlier full state sequence. They
retain their original policy and producer identity. They are not rewritten or
reopened just because the current runner can skip work. Unknown policy versions,
failed/partial records and mismatched requested CP coverage remain ineligible.
New records retain input-identity, state decisions and rebuild checks alongside
the portable report. Reports contain no invented images for omitted states.

The Observatory keeps omitted lane cards with an explicit reason and no process
heartbeat/progress simulation. An empty completed candidate selection says
`No capture candidates found`; discovery/planning remains indeterminate. Rebuild
counters say not performed when omitted. File byte sizes describe their named
artifacts: an unopened `default.xlsx` working copy is not an observed SD state;
use `state_decisions` to interpret it and `input_default` for raw shrink size.

### Observatory display reads

`/api/state` preserves its complete corpus response. The targeted UI requests
`group_scope=live` after the paged workbook index becomes available. This view
includes every active group, the selected workbook and eight recent summaries.
`groups_total` records the complete count. Historical navigation uses
`/api/workbooks` independently of this display projection.

Parsed display snapshots and browser diagnostics have bounded caches. Metadata
changes invalidate snapshot entries, with a forced reread after 30 seconds.
Read errors discard cached entries. Scientific evidence and reuse eligibility
remain owned by the comparison pipeline. Hidden historical cards retain their
records while releasing their canvas and observer resources.

### Optional SQLite diagnostics

`EXCEL_SHRINK_SQLITE_METADATA=1` enables per-owner SQLite diagnostic storage.
Comparison snapshots and resident event batches are committed independently of
scientific result records. Acknowledged event UUIDs are deduplicated, conflicting
reuse fails, and terminal comparison state is committed immediately. Readiness
metrics and watchdog counters retain their existing file contracts. Historical
JSON remains readable and migration never removes source documents or binary
evidence. See `../visual-validation/sqlite-metadata.md` for activation, export
and backup constraints. Diagnostic consolidation does not alter capture scope,
image comparison, exclusion decisions or result reuse eligibility.
