# Portable SheetScope reports

The standalone validation runner separates its disposable workbook working
copies from retained comparison evidence. A packaged comparison can be viewed
after its input workbooks, processed workbooks and external processing programs
have been removed.

## Report boundary

Each comparison lives under `pairs/<id>/` with three kinds of retained data:

- `report.json` preserves the targeted validator's scientific observations,
  coverage, state decisions, XML transitions, limitations and failure status.
  Image references are relative to this directory.
- `evidence/` contains every image referenced by the report, plus generated
  diagnostic masks. Image filenames use the SHA-256 digest of their encoded
  bytes. Identical image files are stored once within a comparison. PNG and
  JPEG chart exports keep their actual format; no image is rescaled.
- `bundle.json` records the SHA-256 digest and byte length of the report and
  each retained image. It is written last and marks successful publication.
  A directory without this manifest is an incomplete package.

Workbook files, resident-host upload pools, XML scratch data and unreferenced
diagnostic files are not copied into the evidence package. Working copies can
only be discarded after packaging succeeds. A missing referenced image is a
packaging failure, including when packaging a failed comparison. It is never
silently converted into a missing image or an equality result.

Successful targeted runs also retain `partial-report.json` from intermediate
publication. A final `report.json` with status `completed_with_limitations` takes
precedence when no `failure.json` exists. Otherwise the partial report is used
when available, and failure context is included. Packaging does not promote an
incomplete scientific observation to a successful comparison. An explicit
failure also permits retaining an intact partial snapshot if a crash truncated
the final JSON; without that failure marker, malformed final JSON is an error.

## Read-only result presentation

The run overview lists workbook outcomes, missing or failed comparisons,
recorded difference counts and explicit reuse provenance. Name search, outcome
filters and pages of 50 entries avoid placing an entire large corpus in the
DOM. Selecting a workbook opens only that comparison's report.

The comparison page provides:

- Region and chart navigation with the recorded comparison states preserved.
- Paired images at a common fit scale or at their original pixel size.
- The Observatory's existing red pixel pulse, hotspot outlines, enlarged crops
  and click-to-inspect pixel view.
- Raw pixel counts, channel-difference measurements and separate unpaired-edge
  observations; these descriptive numbers do not change scientific verdicts.
- Recorded coverage, limitations, state decisions, height checks,
  conditional-formatting decisions, formula diagnostics and XML transitions.
- Links to the scientific JSON and integrity manifest.

The shared `atelierRegions` model accepts a URL resolver. The live Observatory
continues to resolve its evidence API; the portable viewer only accepts the
package's relative, digest-named PNG and JPEG paths. Both use the same
`createPixelInspector` and `pixelSummary` implementations. The static viewer
does not duplicate the scientific selection or pixel-inspection algorithms.

The copied CSS and JavaScript are local assets. Pages embed their JSON data and
load one classic script, so the website works from `file://` as well as from an
ordinary static web server, without an API, Excel or the validation executable.
Workbook labels and JSON data cannot introduce HTML or executable script.

## Pixel diagnostics and limits

Packaging measures differing image pairs serially with the same Rust
`compare_pixels` function used by the live Observatory. Its exact RGBA mask and
the hotspot summary are retained. A bounded cache reuses up to 128 image-pair
diagnostics within one comparison. At most two source images and one diagnostic mask are
decoded at once; images above 32 million pixels are retained but receive an
explicit diagnostic-budget limitation instead of being decoded for this view.

An unavailable diagnostic is displayed as such. The original scientific
comparison, full retained captures and their geometry remain available. Red
overlapping pixels and unmatched capture edges stay distinct; neither a
brightness delta nor a differently sized capture automatically constitutes a
programme regression. Zero-area and missing-chart observations keep their
explicit rendering/presence outcomes without invented partner images.

The read-only website does not expose process controls, live worker telemetry,
clipboard access to original workbooks, new captures or formula queries that
need workbook files. Already recorded formula diagnostics remain in the JSON
and are displayed as structured evidence. The legacy 3D live scene is not part
of this export.

## Publication, reuse and export

`validation_report::package_comparison` is the retention boundary used after
each validation. It also accepts an existing package: its manifest is verified,
then the same immutable evidence and report are copied without rerendering or
recomputing pixel diagnostics. This integrity check establishes that the
retained files match their recorded hashes, not that a chosen OOXML reuse
policy establishes general visual equivalence.

`write_pair_page` publishes a newly completed comparison without reading all
previous comparisons. `write_site` creates the final overview and derived
viewer pages. These functions never rewrite `run.json` or the scientific
`report.json` files. The runner's hydrated run index is the source of the
overview; an interrupted journal can be read without running validation again.

`export_site` copies the hydrated run metadata and verified evidence packages
to a new destination, preserving `identities/<id>.json` when a pair records a
selected-part fingerprint. This lets an exported baseline retain the metadata
needed for the runner's explicit reuse decision. The source report stays
read-only. A recorded `identity_sha256` is checked before that metadata is
copied. The exported journal index is marked complete as a snapshot, while the
scientific run status remains unchanged. The export does not copy original/processed workbook files,
executable programs, live telemetry or scratch logs.

The `run.json` and scientific reports retain their provenance and configuration
metadata, which may include original path strings. Those strings are data;
the portable viewer cannot use them to read the local filesystem. Only
confined relative package assets are loaded. The export is not a redaction
operation.

## Verification handoff

Rust tests in `validation_report/tests.rs` cover source removal after retention,
exact retained masks, JPEG chart exports, final/partial report precedence,
failed evidence publication, corrupt-image reuse, metadata-preserving export
and path confinement. The web test `tests/portable-report.test.js` checks local
asset paths, filtering and unchanged comparison projection across the live and
portable URL resolvers. The latter projection comparison is explicitly a
refactoring safeguard; the confinement and package-integrity tests also guard
ongoing functional requirements.

Native Excel, Windows file/dialog behavior and a browser presentation check
still require verification on the target machine. A generated static page
does not imply that a new Excel validation was performed.
