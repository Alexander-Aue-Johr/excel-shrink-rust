# Targeted corpus comparison

Cell captures use the intersection of each candidate region and the content
envelope, expanded by one row and one column. The envelope is the union of
content-bearing cells in raw Original/Default, ordinary saved controls, Check
Performance and rebuilt copies. Formulas and stored strings count as content;
formatting-only cells do not extend it. This is a bounding rectangle, not a
claim that every cell inside it contains data.

Changed cells and candidate rectangles select a common 128-row × 16-column grid,
reusing the conditional-formatting grid size. Each tile is captured once per state
and clipped to the content envelope. Neighboring tiles remain separate. This
avoids one screenshot per changed cell without limiting candidate or tile counts.
`capture_policy: targeted-content-grid-v2` records this coverage policy explicitly.

Deleted real values therefore remain eligible. Deleted blank tails do not cause
hundreds of thousands of captures. `report.json.content_coverage` records each
sheet's envelope, requested count, skipped count, clipped count and unique
selected regions. Skips mean **outside the chosen coverage**, not visually equal.
Exterior empty-cell XML changes are counted before allocating candidate objects;
each transition records `skipped_outside_content_cell_changes` per sheet. These
counts are per transition, so they must not be summed as unique workbook cells.
Charts are exported separately, including chart sheets, even outside cell bounds.
Late rebuild candidates are backfilled in the retained pre-rebuild workbooks.

Charts with a measured zero width or height have no renderable image. They retain
their geometry and `zero-area-chart` state; `pixels_compared` is false. Collapsing
from a visible chart to zero area is a rendering/geometry difference, not an image
export failure or a successful pixel comparison.

The one-cell margin is a coverage policy, not a proof that arbitrary text overflow
or formatting outside content cannot change. Reports retain this limitation.

Focused native verification:

```powershell
cargo build -p excel-shrink-cli
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/targeted-comparison/run-smoke.ps1 -Output .tmp/targeted-recalc-check -RecalcCandidate
```

Use a new output directory for each native smoke test.

## Corpus commands

The WPF launcher now selects this strategy by default, including when loading an
older machine configuration. Legacy screening flags in those configurations are
ignored while `settings.targeted_comparison` is true. Shrinking prepares Default
only. Check Performance is generated on demand from the saved Original control.
No legacy full visual pass follows the targeted comparison.

```powershell
cargo build --release -p excel-shrink-cli
.\target\release\excel-shrink-cli.exe regression study --request-file .tmp/distributed-study-request.json --output-root .tmp/regression/targeted-study --run-corpus Data.gov --resident-capture --targeted-comparison --control-room --comparison-workers 4 --order size-asc --startup-timeout-seconds 600 --workbook-timeout-seconds 7200
```

Use your machine's existing request/configuration paths; keep its archive binding
and corpus IDs. Add `--targeted-skip-check-performance` for Original/Default only.
`--size-part 1/4` through `4/4` and `--order size-desc` retain their existing meaning.
Do not combine targeted mode with `--preparation-hash-profile`, `--aggressive-first`
or `--full-scan-deferred`. The workflow uses four retained states without Check
Performance and six with it, so concurrent groups multiply that process count.
The example allows two hours for the complete multi-state workflow; individual
Excel operations remain bounded and closing is capped at 15 seconds. Existing
WPF timeout settings remain authoritative. Use a sufficiently long workbook
deadline for the slowest-100 benchmark rather than interpreting valid ongoing
capture work as a stalled Excel command.

Resume the **same output root and arguments** to reuse completed targeted results.
Old visual/hash results are not accepted as completion of this policy. A completed
targeted result remains explicitly limited coverage (`checks_complete: false`).
Namespace exclusions keep their existing reasons. When Check Performance is
requested, an unchanged Default does not skip that independent optimizer lane.
Proven nonvisual Default-only exclusions remain available when CP is omitted.

Existing prepared bundles can be reused with `--reuse-prepared-from <full executable
SHA-256>` after reviewing shrink compatibility. Keep the associated records and
source catalogs; copying a ZIP alone is insufficient. Use `--compare-only` when
all required preparations are already present. Never reuse old comparison verdicts
as benchmark results. The local slowest-100 corpus and its baseline remain outside Git.

Durable evidence contains `portable-report.json`, cell PNGs and converted chart PNGs.
These follow records through the existing `--export-results` / `--import-results`
verified transfer. Machine/run attribution remains separate for preparation and
comparison. Completed temporary XLSX states are removed to bound disk use; raw
Original/Default remain recoverable from preparation bundles. Native smoke tests
retain their states for inspection.

All verdicts use original-resolution images. After the complete comparison,
confirmed-equal cell tiles use the existing `equal_preview_width` setting (default
512 pixels; 0 retains full resolution). Different tiles and their eight captured
neighbors retain full resolution. Charts retain full resolution. Byte-identical
images share one retained file. The JSON records original image dimensions and
SHA-256 separately from retained preview dimensions. Reports are published before
superseded images are removed; reducing evidence never changes comparison verdicts.

## Compact XML audit evidence

Transitions contain `change_count` and `changes_file`, rather than millions of
inline change objects. Each `xml-changes/NN.jsonl.gz` stores the complete audit
once. The first JSON line declares schema 1, column names, and shared context
strings; subsequent lines are arrays in that column order. The two context
columns index the header's context table. Opening/save overlap remains explicit
per change. No changes are sampled or discarded. Audit files follow the portable
report through verified result export/import.

The comparison validates immutable image contents once and records
`image_comparison_statistics`. Encoded duplicates reuse validated evidence;
different encodings are compared using dimensions and SHA-256 of normalized
RGBA8 pixels. Missing or invalid images cannot establish equality. Only small
fingerprints are cached, not full decoded images.

Existing start commands remain valid; these optimizations require no new flags.

Targeted screenshots use authenticated binary HTTP uploads to the existing Rust
resident host. Only an image receipt and geometry enter the command-response JSON.
The native bridge moves the owned uploaded image to its evidence path; it no longer
decodes or retains a second Base64 copy. Chart exports may be PNG or JPEG even when
Office documents a PNG, so the host checks the actual format. Failed uploads and
missing receipts remain failures; zero-area charts retain their explicit state.

Cell capture batches now submit one request per available Excel state concurrently.
Each lane still has at most one outstanding request. The native bridge uses file
notifications with bounded rescans rather than a fixed sleep after every capture.
Late candidates are still backfilled in all retained states. Once final discovery
is complete, Excel is closed before disk-only comparison and reporting.

For a direct A/B diagnostic, add `--serial-captures` to `regression targeted-compare`.
The corpus runner uses parallel capture by default; existing commands need no
changes. `timings_ms` separates capture selection/transport from staged image
comparison, and `image_comparison_statistics` records actual decoding/reuse.
Capture counters are coalesced to 250 ms; phase transitions publish immediately.

Hash-only client retention is deliberately not the default: the Original's
immutable pixels may be needed only after later rebuilds reveal new differences.
Discarding them early would require unsafe recapture, while retaining all of them
in WebView memory would grow with sheet size. Binary uploads, host fingerprints,
and post-comparison equal-image reduction keep that evidence on disk safely.

XML snapshots are reused only for identical complete archive SHA-256 values.
Saving or rebuilding a path changes the key and forces a new analysis. Reports
record `xml_snapshot_statistics`; cell streams for retained snapshots live in
the attempt's `xml-scratch` directory and are removed when their final owner exits.
This avoids repeated decompression/parsing without caching mutable filenames or
accumulating large fingerprint files on the Windows system volume.

Final Check Performance change lists contain selected capture regions, marked by
`change_coordinates: "content_clipped_capture_regions"`. Original candidate counts
remain explicit; every original change coordinate stays in its transition audit.
Generated report JSON is compact; progress telemetry and maintained source remain
formatted for human reading.
