# Static Sheet Atlas

Every completed `deltasieve run` writes `index.html` as a static Sheet Atlas.
`report.html` contains the detailed retained-report viewer. Pair pages and their
scientific JSON, integrity manifests and images remain available for inspection.

The Atlas uses the visual design, card painter and comparison controls of
Sheet Atlas revision `d10a25de9491c19a87d09d1f992afd6594557eaf`.
Three.js r180 is vendored locally with its MIT license. All executable browser
assets are embedded in DeltaSieve and copied during export. Serving the result
requires static HTTP hosting, without a JavaScript build or application backend.

## Label new runs

Add presentation labels to a normal run configuration.

```json
{
  "atlas": {
    "corpus": "Destatis stress panel",
    "variant": "SheetShrink e5e4fe8"
  }
}
```

These fields accompany the existing configuration. Each processor and version
still has an independent run, input identity, environment and comparison policy.
Selecting variants changes the displayed findings without rerunning comparisons.
Original open/save/rebuild control comparisons stay in the detailed report.
Atlas processor markings use Original-versus-candidate comparisons (`O`/`OR`
against `SD`/`SDR`). They do not attribute an Original control's drift to a cleaner.

## Combine runs and label existing evidence

```powershell
deltasieve merge C:/studies/nx/report C:/studies/check-performance/report `
    C:/studies/xlstyles/report C:/studies/sheetshrink/report `
    --output C:/studies/collection
deltasieve export C:/studies/collection C:/studies/atlas-site `
    --labels C:/studies/atlas-labels.json
deltasieve serve C:/studies/atlas-site --port 8787
```

Merge and export destinations must be new directories. Labels for existing runs
are supplied separately, keyed by the `run_id` in each source `run.json`.

```json
{
  "source-run-id-for-nx": {
    "corpus": "Destatis stress panel",
    "variant": "NXPowerLite 11.0.1 Balanced"
  },
  "source-run-id-for-sheetshrink": {
    "corpus": "Destatis stress panel",
    "variant": "SheetShrink e5e4fe8"
  }
}
```

The label file is copied as `atlas-labels.json` and survives subsequent export.
`deltasieve atlas C:/studies/atlas-site --labels C:/studies/atlas-labels.json`
refreshes an existing Atlas in place without copying its retained evidence.
It changes derived presentation metadata. Source reports and their package
hashes are preserved. Unknown labels display as `Unclassified` and `Candidate`.
Distinct run identities are retained even when labels match. Nested merges
record their previous source references so the Atlas can resolve original labels.

Workbooks are grouped by declared corpus, relative workbook name and original
SHA-256. Complete package fingerprints provide a fallback for older reports.
Missing identity evidence keeps observations separate.

## Continuous navigation

Workbook groups contain persistent Sheet objects. Camera movement changes the
view over this same hierarchy. Worksheet order, slot position and base marking
survive navigation and processor selection. Unchanged sheets stay in the grid.
Clicking a workbook loads small previews into its existing sheet cards. Opening
a sheet loads its compressed comparison details and adds the retained capture,
finding outlines and folded coordinate map. Additional imagery fades in.

The flat CSS renderer uses the same persistent hierarchy and camera coordinates.
It can be selected explicitly, including when WebGL is unavailable. Sidebar
buttons provide keyboard access to every known workbook, sheet and finding.
Reduced motion disables the camera interpolation and CSS transitions.

The complete workbook inventory includes empty, hidden and chart sheets in
original order. New comparisons retain it before Excel opens the workbook.
Older reports can recover it from a still-available configured source directory.
When that source is absent, only known sheets appear and the inventory is marked
incomplete. Envelope row/column counts describe the recorded content envelope,
including its context margin.

## Evidence and image coverage

Red records the existing pixel, geometry, presence or rendering difference flags.
Dashed cards identify incomplete observations. The sidebar also lists failed
and unattempted processor runs. Neutral cards inherit the coverage limits of
their source reports.

Thumbnails are derived from retained Original images. The folded map describes
the union of recorded capture coordinates. Uncaptured imagery stays neutral.
It does not construct screenshots for unobserved regions or imply full-sheet
rendering. A worksheet can therefore have a valid inventory entry and no image.

Multiple processors share a comparison context only when the retained Original
image identity, range, native state, Original geometry and validation signature
match. Other processors are listed as lacking a matching retained context.
Native PNG/JPEG pixels are drawn without rescaling into the inspection canvases.
Different image extents have transparent padding. Recorded geometry and ignored
blank capture extents remain separately identified in the pane notes.
Exact grouping, difference masks and connected magnifiers describe these images.
They do not replace the validator's verdict or its coverage policy.
Comparison title areas are green for recorded agreement and the Original
reference, red for recorded differences, and neutral for unassessed outcomes.
A grouped title takes the difference outcome if any member has a recorded
difference, including geometry or presence differences with matching pixels.

The browser bounds compressed and expanded JSON reads and native image inspection.
The combined inspection canvases are limited to 32 megapixels. Oversized or
missing captures retain a link to their detailed source report.

## GitHub Pages

Publish the entire exported `atlas-site` directory as the Pages artifact.
The export contains `.nojekyll`, local JavaScript/CSS/Three.js assets, gzip JSON
data, PNG previews and retained evidence. Paths work under a repository subpath.
The browser detects gzip bytes and decompresses them with `DecompressionStream`.
It also accepts data already decoded by an HTTP `Content-Encoding` header.
Images load through ordinary relative static URLs.

The catalog records the directory size before the final index write and uses
a conservative 1,000,000,000-byte Pages budget with 1 MiB reserved for metadata.
An over-budget export is identified in the footer. Lazy loading reduces the
initial transfer, while the complete publication still needs to fit the host.
The detailed viewer retains its existing publication-review inventory.
[GitHub Pages limits](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits)
[Browser decompression](https://developer.mozilla.org/en-US/docs/Web/API/DecompressionStream)

For local review, `deltasieve serve` binds a read-only server to loopback and
opens the browser. `--no-open` suppresses browser launch. Ctrl+C stops the server.
The `open` command opens the self-contained detailed report for direct file use.

## Checks

```powershell
cargo test -p excel-shrink-cli --lib validation_report
cargo test -p excel-shrink-cli --lib inventory_tests
node --test excel-shrink-cli/office-capture/sheet-atlas/tests/model.test.mjs
```

The focused contracts cover independent variants, complete blank-sheet inventory,
nested source labels, failed legacy runs, source-package integrity, gzip transport,
context matching, complete slot layout and unusual worksheet names.
