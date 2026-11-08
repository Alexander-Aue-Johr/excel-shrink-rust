# Portable preparation artifacts

This contract defines the next integration step after the persistent-worker
acceptance experiment. The explicit-input and source-manifest preparation CLI
interfaces are implemented. Package publication and import preview remain
implementation requirements until their acceptance evidence is recorded.

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
blob is present and verified. Build it in a new sibling staging directory and
publish to a new destination. Existing packages remain immutable. A failed
export retains its explicit incomplete status and cannot replace a complete
package. Archive wrapping is transport; extracted package contents undergo the
same validation.

## Import preview and use

Preview the exact selected source set with paged details and aggregate counts.
Classify each item as complete, missing, unsuccessful, invalid, baseline mismatch
or ambiguous generation. Preserve candidate generations with their provenance
and require an explicit generation choice where matching is ambiguous.

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
SheetShrink evaluation can explicitly adopt the same packaged baseline where
needed. Native generation, artifact import and visual verdicts retain separate
lifecycles and reuse checks.

## Acceptance gates

Test valid transfer, relocation, partial coverage, failed inputs, hash corruption,
source/baseline changes, duplicate occurrences, multiple generations, changed
partitions, producer/consumer environment differences and unavailable local
Check Performance. Test interrupted publication and retries without overwriting
completed evidence. Verify that measured producer costs remain attributable to
the producing machine and that the normal comparison entry point can consume
the imported pairs. Keep Enron advancing during bounded native acceptance tests.
