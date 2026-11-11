# Publishing a DeltaSieve result snapshot

The static explorer uses retained report JSON, relative image paths, and bundled
browser scripts. It supports ordinary HTTP hosting without a validation runner,
Excel installation, or control API. Opening `index.html` locally also keeps the
embedded report data available. Each comparison's integrity manifest covers its
scientific report and retained images.

## Retain rendering observations

Pixel differences in charts remain visible findings even when follow-up controls
attribute them to native rendering variability. Retain both images, exact pixel
measurements and the original comparison outcome. A small changed-pixel count
does not suppress the finding. The report describes an observed visual change;
attribution to a SheetShrink regression requires separate evidence.

Document repeated original-versus-original and original-versus-output controls
alongside the finding when available. An unreproduced small discrepancy may be
accepted as a rendering observation while remaining in the report. Unchanged
chart XML alone does not prove unchanged dependent values, layout or rendering.

## Freeze a private export

Use the standalone report root produced by DeltaSieve. Corpus studies first need
their existing retained comparisons imported into that format. A source study's
working-directory tree is unsuitable as a web root.

```powershell
& 'V:\deltasieve-build\debug\deltasieve.exe' export `
    'V:\deltasieve-data\completed-report' `
    'G:\deltasieve-releases\enron-2026-11-08-buildid'
```

The destination must be a new directory outside the source report. Export verifies
package checksums and copies referenced evidence. It preserves incomplete and
failed outcomes. `publication-review.json` lists metadata concerns and the overview
displays its review requirement. This operation leaves the source report intact.

## Review the material to be published

Inspect every flagged metadata location and decide whether the observation may be
shared. Review workbook names, formulas, image contents, machine descriptions,
corpus rights, and anonymity requirements separately. The inventory is heuristic
and does not detect arbitrary secrets inside workbook content or images.

Do not edit a report underneath an existing integrity manifest. A future redacted
package needs an explicit derivation record, new checksums, and verification of its
retained observations. Keep the current private export until that review succeeds.
Public exports should identify CPU, core count, RAM, Excel build and method version
using an approved environment description. A machine's personal hostname is
unnecessary for that purpose.

Record the selected corpus manifest, software commit, producing binary checksum,
configuration, coverage cutoff, outcome denominators and unresolved findings in
the release notes. Keep the exact original source and raw evidence privately when
distribution rights prevent releasing them. Record such omissions explicitly.

## Serve and inspect the approved snapshot

For a reviewer-facing export of a completed static Atlas, generate the
publication directory from the private Atlas:

```powershell
& 'V:\deltasieve-build\debug\deltasieve.exe' public-atlas `
    'G:\deltasieve-releases\private-atlas' `
    'G:\deltasieve-releases\reviewer-atlas'
```

This directory keeps the comparison pages, report observations and retained
images. The native configuration and replay archives stay in the private
export. Use `public-provenance.json` to trace the displayed observations to
their source reports. Wait for `.incomplete` to disappear and review
`publication-review.json` before serving the directory.

Check the size of the complete reviewer directory against the hosting budget.
The Atlas displays its estimated size and whether it exceeds the configured
GitHub Pages budget. A larger release requires a static host with sufficient
capacity. Verify the complete directory on the intended host so that both
the overview and its linked native image evidence remain available.

```powershell
python -m http.server 18890 --bind 127.0.0.1 `
    --directory 'G:\deltasieve-releases\enron-2026-11-08-buildid'
```

Inspect the overview, a differing comparison, pixel details, incomplete outcomes,
and relative image links. Stop this test server afterward. The validation runner
is independent of this read-only HTTP server and is unnecessary for the viewer.

Place the approved directory under an immutable GitHub Pages path such as
`runs/enron/2026-11-08-buildid/`. Include an anonymous hardware label in its metadata
and use a stable landing page to link releases. Preserve all relative directory
relationships. The exported site has no external package-loader dependency and
does not fetch private API endpoints. Publish only reviewed artifacts, leaving
corpus ZIPs, workbooks, build trees and telemetry scratch files outside that tree.

Generate a release-level SHA-256 inventory before upload and verify the hosted
downloads against it. Keep the release immutable so a paper's link continues to
identify the evaluated snapshot. The publication destination and its access
policy remain deployment choices.
# Live capture inspection and retained equal images

New configurations retain verified-equal images by default. Setting
`capture.retain_equal_images` to `false` explicitly keeps the discard policy.
Saved configurations retain their explicit choice.

Packaging may recompress verified-equal PNG images with lossless PNG DEFLATE,
the encoder's best compression setting and adaptive filters. It accepts the new
encoding only when it is smaller and its decoded RGBA pixels exactly match the
capture. Images referenced by differing or unverified observations preserve their
original encoding. Integrity manifests describe the final retained bytes.

The live Sheet Atlas reads bounded image receipts from the active comparison.
Worker signal timelines put capture glyphs on the heartbeat line. Capture glyphs
take visual precedence over nearby heartbeat pulses, while recorded heartbeat
events remain intact. Clicking a glyph opens that receipt in the live Atlas.
The selected spatial view stays pinned when later receipts evict older sheets
from the bounded history. Enabling follow mode resumes the live sheet updates.
The live view reuses the Atlas spatial renderer, shows available native states,
and labels them as received captures without claiming a comparison verdict.
Recent inspection retains at most 128 receipts per native state. Completed static
reports and merged reports remain independent of this live inspection window.
