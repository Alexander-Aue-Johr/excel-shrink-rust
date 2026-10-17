# Local asset delivery: capture startup correction

## Incident

Run `20261015T001121830Z-ad82f7cc` reached all eight readiness cases but produced
no captures. Each .NET host returned HTTP 404 for `/capture-geometry.js`.
The contextual-capture patch shipped and loaded `capture-geometry.js` and
`context-validation.js`, but did not add the two allow-listed routes to either
native host. The second missing route was masked by the first failed load.
This is a deployment/integration error, not evidence about Excel geometry,
tiling, image quality, host speed or the megapixel budget.

## Correction

Both hosts explicitly serve the two scripts using their existing static-file
helper and JavaScript content type. No generic directory listing or arbitrary
file-serving endpoint is added.

The shared Office.js runner now performs a local web-asset preflight after the
host health check and before developer registration and workbook launching:

- Check every required staged file for existence and a bounded nonzero length.
- Fetch `/config` through the actual task-pane origin and verify the run ID,
  experiment and host kind.
- GET required scripts, HTML, CSS, generated capture options and the active
  `mode.js` through that same origin.
- Require HTTP 200, expected Content-Type, exact byte length and exact SHA-256
  equality with the staged file.
- Keep OS TLS validation enabled. Disable redirects and proxies for these
  loopback requests. Bound each response and request time.
- Write `asset-preflight.json` at the case root on success and failure. The
  failed route, HTTP status when available, phase and cause remain visible.

The preflight covers local deployment only. It does not load the external
Microsoft Office.js CDN or execute scripts in Excel. It does not prove the
new geometric and context algorithms correct.

A failed shared-asset preflight stops the final qualification queue after its
first affected case. Remaining readiness cases are marked `blocked`, not
measured. `infrastructure-blocked` is distinct from an ordinary quality
failure. Normal worksheet geometry and pixel failures retain their existing
behavior and do not trigger this deployment classifier.

## Run

Apply the Python installer after `patch_visual_validation_context_capture.py`.
Then run the existing VS Code entry:

**Visual validation: Finalize capture strategy**

No new task, manual cache deletion, file unblocking, DLL installation, certificate
work or editing of workbook inputs is needed. Host source changes already feed
the existing build fingerprint, so that runner rebuilds its affected host.
Old report archives remain unchanged. The corrected source uses a new campaign
identity under the existing no-mixed-source-results policy.

Upload the one `review-metadata.zip` from that run. Asset checks are included in
that bundle. No images are required to diagnose a failed preflight.

## Deliberately unchanged

No geometry factors, context margins, size tolerances, tile budgets, pixel
comparison rules, input workbook settings or production regression baselines
are changed. No old measurement is relabeled as successful. Package versions
and their audit warnings are not altered by this narrow fix.

## Tests and limits

The patch includes 15 pure Python routing-contract and queue-classification
checks. The route-removal mutation test reproduces both missing routes.
Run with:

```text
python -m unittest discover -s docs/visual-validation/experiments/tests -p test_local_web_assets.py
```

Those checks do not compile C#/Rust, run PowerShell, make an HTTP request or
open Excel. Native verification occurs on the user's machine through the
preflight followed by the original contextual capture qualification.
