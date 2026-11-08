# Portable preparation artifacts

This contract defines the next integration step after the persistent-worker
acceptance experiment. The explicit-input and source-manifest preparation CLI
interfaces, package export and read-only package verification are implemented.
Selection coverage preview and strict paired-folder import are also available
through the CLI, including explicit baseline adoption and generation selection.
The local web configurator exposes package transfer and native preparation-job
controls for source manifests. Full visual browser acceptance remains open.

## Source identity and selection

A preparation input has a stable source ID, a logical display name, a local
baseline path and its SHA-256. A known raw source additionally records its hash,
format and corpus-member reference. Corpus studies reuse their existing
`corpus_id:member_index:member_name` identities. Duplicate archive occurrences
retain distinct member indices and remain in the denominator.

The input manifest resolves relative baseline paths against its own directory.
It verifies each expected baseline hash before native processing. Source IDs
must be unique within the selection. Logical names remain metadata and cannot
select arbitrary output paths. Native generation preserves the source files.
Raw-source references are supplied by the manifest producer. The baseline hash
is verified locally; conversion provenance retains its upstream evidence.
Each job retains a byte-verified `baseline.xlsx` and opens that private copy
read-only in Excel. Its receipt records the relative baseline filename, length
and hash, allowing later export after the external input path has moved.
Each successful per-input receipt also retains its producer executable SHA-256,
so an interrupted batch does not remove that provenance. Package export prefers
the per-input producer and uses the batch producer only for older receipts that
lack the field. A present malformed producer identity fails export. Mixed
generations retain distinct producer records instead of being relabelled as the
current batch's producer. This metadata support does not itself implement
automatic interrupted-batch recovery.

Use `--source-manifest sources.json` instead of `--input` for corpus preparation.
Version one contains `version` and an `inputs` array. Each input requires
`source_id`, `name`, `baseline` and `baseline_sha256`. Optional `raw_source`
contains `sha256`, `format` and an optional `member` with `corpus_id`, `index`
and `name`. The reader accepts UTF-8 with or without a BOM. Missing XLSX paths
remain selected so their job outcomes report failure instead of dropping them.
An invalid manifest version, duplicate ID or invalid hash syntax rejects the
configuration before native work starts.

An explicit file list remains supported for small experiments. Its generated
occurrence identities are scoped to that list and do not establish a mapping
to a different corpus selection. Cross-machine corpus reuse requires the
stable source identities supplied by the corpus manifest.

Partition count and index describe the selection. They are excluded from
per-source artifact identity, so compatible preparations survive a new partition
layout. A selection digest records the exact selected source IDs. Existing
size-based study partitions and standalone name-hash partitions must identify
their scheme explicitly until the unified selector replaces the separate paths.

## Prepared states

The package records these roles separately where available.

- The exact prepared XLSX baseline used as input to the operation.
- Saved-Original control, with Save or SaveAs method and working-copy semantics.
- Before-optimization and after-optimization controls.
- Optional rebuilt states, their exact parents and rebuild method.

Confirmed optimization and confirmed absence of work are distinct successful
native outcomes. Failures and unsupported inputs retain their reasons. Missing
controls cannot be inferred from a success label. Successful generation leaves
visual validation pending.

For an XLS source, the conversion baseline belongs to its conversion provenance.
If a receiving machine produced a different baseline, the import preview reports
that mismatch. The operator can explicitly use the packaged baseline for the
comparison workflow. Local conversion measurements and producer conversion
measurements remain separate. This also keeps both comparison variants derived
from the same baseline when Excel versions serialize XLS conversion differently.

## Package representation

Use a small versioned `manifest.json`, an indexed SQLite metadata database and
content-addressed XLSX blobs. The header binds the closed database by hash and
records counts and package policy. Metadata records source occurrences, state
roles, native outcomes, provenance references, measurements and selection.
Repeated producing environments and recipes are stored once by content hash.

Blob filenames are constructed from validated lowercase SHA-256 values.
Identical bytes may share a blob while source occurrences, generations and
measurements remain distinct. A difference between generations remains visible.
The transfer package avoids adding one diagnostic JSON file per event or input.
The existing worker's raw receipts can remain in its private working directory.

Publish a package only after database transactions close and every referenced
blob is present and verified. Reserve a new destination exclusively, retain an
incomplete marker during construction and publish the manifest last as the
completion marker. Readers require that final manifest. Existing packages remain immutable. A failed
export retains its explicit incomplete status and cannot replace a complete
package. Archive wrapping is transport; extracted package contents undergo the
same validation.

## Import preview and use

`deltasieve preparation-verify PACKAGE_ROOT` verifies a transferred package
without Excel. It requires a complete publication marker, supported policy and
database schema, a matching database hash, SQLite integrity, matching blob
hashes and lengths, complete state references, generation and provenance
bindings, native completion evidence and reconciled outcome counts. It opens
SQLite read-only and does not rely on the producer's original local paths.
These integrity checks do not authenticate the producing party.

Preview the exact selected source set with paged details and aggregate counts.
Classify each item as complete, missing, unsuccessful, invalid, baseline mismatch
or ambiguous generation. Preserve candidate generations with their provenance
and require an explicit generation choice where matching is ambiguous.

Run `deltasieve preparation-preview PACKAGE_ROOT --source-manifest sources.json`
with optional `--offset` and `--limit` (1–1000, default 100). This read-only
operation verifies the entire package first and rejects invalid packages rather
than presenting corrupt entries as usable. It then reconciles all selected
occurrences, independent of the detail page. Output includes the selection
digest, aggregate status counts and candidate generation/provenance identities.
Only source-manifest packages support corpus reuse. An explicit-file-list
package does not establish a transferable corpus mapping.

Complete coverage requires one compatible successful generation, an exact
baseline hash and matching raw-source provenance when supplied by the selection.
Multiple compatible generations are ambiguous. Missing and failed preparations,
source and baseline mismatches, and unavailable or changed local baseline files
receive separate outcomes. A missing local baseline is not silently replaced by
the packaged copy. Explicit adoption belongs to the subsequent import workflow.

Verify schema, database and blob hashes, lengths, source identity, baseline,
expected state roles and native outcome. Resolve metadata paths safely, reject
reparse-point escapes and never derive filesystem destinations from display
names. Unknown schema versions fail explicitly. Integrity hashes make transport
corruption detectable; producing provenance remains separately inspectable.

Imported-artifact mode performs no local Check Performance operation. Missing or
invalid entries remain visible for later preparation. The producer's Excel
binary/version, culture, recipe and machine provenance remain attached to its
states. Local opening, rebuilding and capture measurements receive their own
local provenance. A local renderer change is relevant to comparison evidence,
while a SheetShrink-only update does not invalidate independent CP generation.

The receiver can materialize paired baseline/optimized folders with matching
safe names and an identity index for the ordinary DeltaSieve comparison path.

Use `deltasieve preparation-import PACKAGE_ROOT --source-manifest sources.json
--output NEW_DIRECTORY` for strict import. Every selected occurrence must have
exactly one compatible generation and an unchanged, available local baseline.
The importer uses the same decision as the coverage preview and rejects the
entire request before creating output if any selected occurrence is unresolved.
Select an explicit smaller source manifest when importing a completed subset.
For ambiguous generations, supply `--generation-choices choices.json`. The
version-one file contains `version: 1` and a `generations` object mapping exact
selected source IDs to generation SHA-256 values from the coverage preview.
Unknown sources, invalid hashes and generations belonging to another occurrence
fail before output creation. An unspecified ambiguous occurrence stays unresolved.

`--adopt-packaged-baseline` explicitly selects the package's verified baseline
without requiring the local converted file to be available or byte-identical.
If the selected baseline hash differs from the packaged one, raw-source
provenance must be supplied and match exactly. Source identity checks remain
active. Multiple eligible generations still require an explicit choice. This
option never overwrites the local baseline. The import index records the
requested baseline hash, the chosen generation and import options separately
from the producer's prepared-state evidence.

The new directory contains `baseline` and `check-performance` folders, a
`compare.json` configuration and a final `import.json` index. Pair filenames
derive from the source ID's SHA-256, preserving distinct occurrences even when
their display names or bytes repeat. Every copied state is hash- and
length-verified. The index retains original names, source and generation IDs,
full prepared-state receipts, producer measurements and deduplicated provenance.
The comparison configuration uses the ordinary two-corpus path with no local
optimization stage. Import leaves visual validation pending. Existing output
directories are immutable, and failed construction retains an incomplete marker.

Generated configurations bind `preparation_index` to `import.json`, resolved
relative to the configuration file. This option requires two supplied source
corpora and no processing stages. The run checks the complete source-name set
against the index before partition selection, then verifies both selected
workbook files against their imported state hashes and lengths before rendering
or observation reuse. An altered pair fails explicitly.

The run header retains the import-index digest, package/selection identities,
import options and deduplicated producer provenance. Pair records retain the
original display name, stable source ID, selected generation, state identities
and producer measurements separately from local validation evidence. Technical
pair names and observation reuse keys remain stable. Static report rows use the
original display name and carry the preparation metadata. Static export retains
these fields through the run records, without requiring the source workbooks.

SheetShrink evaluation can explicitly adopt the same packaged baseline where
needed. Native generation, artifact import and visual verdicts retain separate
lifecycles and reuse checks.

## Local web package controls

The separate Prepare Check Performance variants workflow accepts a source
manifest, a new output directory and an independent positive worker target.
POST `/api/preparation/start` freezes the source selection with absolute baseline
paths and retains its hash plus a job receipt under the output parent's
`.preparation-jobs` directory. The job does not derive its selection from mutable
form controls after admission. It calls the same continuously refilled pool as
the CLI and preserves its 512 MiB admission reserve.

Admission and launch run together in a background operation, so disconnecting
the HTTP caller during selection freezing cannot leave an admitted job without
an executor. The room's reservation is held until native execution and terminal
receipt handling finish. `/api/state` includes compact job metadata and reads at
most 64 KiB of preparation counters. The page displays preparation progress
outside the configuration dialog and keeps producer failures explicit.

The owner process must remain running. Reloading or closing the browser does not
stop the job. After interruption, select the same source manifest, a new output
directory and the earlier preparation in the optional reuse field. Changing the
worker target during a running job remains an integration requirement. Completed native
outcomes and job receipts are retained even though the room currently exposes
only its most recent preparation job.

## Selective preparation continuation

The CLI accepts `regression performance-prepare --source-manifest sources.json
--output NEW_DIRECTORY --reuse-from PREVIOUS_DIRECTORY --workers N`. Web jobs
offer the same optional prior-directory field. Continuation copies successful,
compatible inputs into the new batch and leaves every previous file unchanged.
It requires a recorded stopped pool or a positively departed PID/creation-time
owner. Without a stopped pool, a live or unverifiable previous owner is rejected
before creating output.

Completed inputs are matched by stable source ID. Reuse checks the native recipe
asset hashes, positive optimization outcome, supplied raw-source provenance,
current and expected baseline hashes, retained baseline binding, exact control
set and every artifact's length/hash. Missing or incompatible input evidence
enters the native queue with a recorded reuse-miss reason. An unreadable or
ambiguous prior source index fails explicitly. Copy-time corruption also fails
instead of publishing a falsely completed input.

The original producer, environment and measured preparation costs stay attached
to each copied receipt. Reuse is a historical-artifact choice and does not claim
that the current Excel environment generated those states. Different new inputs
may receive new producer provenance. The current executable alone does not
invalidate old CP artifacts. A native recipe change does. Older receipts may
take producer identity from their existing final batch ledger; interrupted old
receipts without either producer source cannot be silently attributed.

Progress and final ledgers report reused counts separately. Native worker count
is based on the remaining queue, so a fully reusable batch starts no Excel
worker. Stable source manifests permit changed order and partitions without
renumbering source identities. Explicit-file-list identities retain their
previously documented list-local scope. This is explicit continuation into a new
batch, with preserved earlier attempts; automatic in-place job reattachment is
still outside the implemented startup path.

Check Performance preparation publishes a compact `progress.json` after pool
initialization, at most once per second while running, and after the terminal
outcome ledger. It records total, claimed, prepared, failed, queued, in-flight
and unresolved counts plus requested concurrency and elapsed time. In-flight
includes claimed requests waiting for worker admission; it is not the count of
live Excel processes. Native worker ownership remains in the pool's evidence.
A stale progress snapshot alone does not establish process liveness. Status
queries can read this bounded counter document without scanning input receipts.
Publication failure aborts the pool before leaving the manager loop so clients
cannot remain waiting on an unserviced pool.

The configuration workflow includes Check Performance package transfer. It uses
the same package functions as the CLI through token-protected local POST routes
`/api/preparation/preview`, `/api/preparation/import` and
`/api/preparation/export`. File verification and copying run on blocking workers
with the room's exclusive operation reservation. Status queries remain separate.
The operation retains its reservation if its HTTP caller disconnects.

Coverage details are paged in groups of 100. Candidate identities are displayed
as text, with generation-choice and explicit baseline-adoption controls for
import. Input changes invalidate the previously imported configuration action.
Import returns a configuration for separate review and start; it does not open
Excel or start validation. A supplied prepared identity index is visible and
editable in the ordinary comparison configuration. Export preserves native
failed outcomes as part of the package denominator.

## Remaining acceptance gates

Test valid transfer, relocation, partial coverage, failed inputs, hash corruption,
source/baseline changes, duplicate occurrences, multiple generations, changed
partitions, producer/consumer environment differences and unavailable local
Check Performance. Test interrupted publication and retries without overwriting
completed evidence. Verify that measured producer costs remain attributable to
the producing machine and that the normal comparison entry point can consume
the imported pairs. Keep Enron advancing during bounded native acceptance tests.
