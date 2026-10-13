# Gridless capture, final image exports, and local HTTP host

Status: experimental capture policy `gridless-final-png-v1` (2026-10-05).
Scope: the Office.js **Get picture / range-image** experiment. Production regression
policy, Rust OOXML content/drawing bounds, VBA capture, and the dedicated row-height,
units and marker diagnostic fixtures are not changed by this patch.

## View normalization

Before any calibration or screenshot, the range-image add-in disables
`Worksheet.showGridlines` for the visible worksheet catalog and reads it back.
Original settings and confirmation are persisted in `safe_capture.capture_view`.
The batch fails if the setting cannot be confirmed. Styled borders, fills, fonts,
zoom, sheet activation and cell values are not altered by this normalization.
The runner opens a disposable workbook and discards it at close; input files are
not saved. This changes the screenshot policy relative to old grid-visible runs.
Do not compare normalized and old unnormalized screenshots as regression pairs.
Both A/B arms use the same normalization.

## Scan bounds are not export bounds

Strategy F previously initialized its probe with eight visible columns, not a
measured 1920-pixel buffer. The new planning step honors `OverflowProbeWidthPx`
(default 1920) using measured column edges and the sheet's calibrated PNG scale.
It rounds outward to a column boundary and records requested/actual pixels and
columns under `probe_buffer` and the `measured_probe_buffer` event. The existing
maximum of 64 extra visible columns is retained and insufficient coverage is
reported. The 16/32/64 MP area budget and 30000-pixel edge budget still both apply.
A wider buffer can require more short row strips even when it saves later scans.
No Excel UsedRange/value scan replaces the Rust plan.

At sheet completion:

| Evidence | Export |
| --- | --- |
| Complete, fully checked, no overflow signal | Remove the right probe margin to the Rust/add-in base bound. |
| Overflow signal in any accepted tile | Retain the entire last scan buffer, consistently for all sheet tiles. |
| Inconclusive, unchecked, horizontal context split or failed | Retain available evidence; never infer a crop from missing results. |

Final PNGs are physically cropped **per bounded tile on the active native host**.
.NET uses ImageSharp and Rust uses its existing `image` crate. This is a rectangle
crop, not a resize and not another `getImage` call. It never joins an entire large
worksheet into one bitmap. Hashes and dimensions are verified before decoding;
the selected capture pixel budget also bounds the crop input. One export worker
runs at a time. Failed exports do not cause native capture retries.

The original capture remains immutable evidence. Cropped files have a `-final`
suffix; final tile metadata records the new SHA-256, dimensions and per-tile range,
plus `source_id`, `raw_png`, `raw_sha256`, `capture_address`, `source_rect`,
`source_geometry`, and `crop_rect_px`. `logical_address` describes the complete
worksheet image extent. The report's Final gallery uses the final PNG; a direct
link opens its raw capture with the probe buffer. Existing positive-overflow images
are reused without a duplicate file. Native export cost is `finalize_ms` and does
not count as Excel rendering time.

"No signal" retains the existing experiment's operational meaning; a differential
blocker test only rules out blocker-sensitive overflow in the verified region. It
does not prove every nontext decoration outside base bounds is irrelevant.

## Rust host transport

The Rust host is now an **HTTP-only local desktop experimental host**:

- Web: `http://localhost:3000`, listener bound to `127.0.0.1:3000`.
- Control: `http://127.0.0.1:3001`, listener bound to `127.0.0.1:3001`.
- No certificate generation, trust-store writes, TLS server or certificate CLI.
- A separate Rust HTTP add-in identity/registration directory avoids reusing the
  cached HTTPS manifest. The staged manifest is archived under `inputs/host/`.
- Host and Origin checks reject non-loopback or cross-origin browser requests.
  Existing capture run-ID checks protect write endpoints including final export.
- The Microsoft Office.js CDN remains HTTPS. The .NET host and its HTTPS certificate
  handling are unchanged. No existing certificates or OS security policy are removed.
- With the Excel /x launch option, the Rust host does not create another COM Excel
  application just for idle control endpoints. The explicit rust-com option remains.

Microsoft strongly recommends HTTPS. Its manifest documentation does not require
HTTPS in every add-in scenario, but warns that HTTP can cause warnings/errors;
Office on the web and Marketplace scenarios require HTTPS. This path therefore
implements the user's local desktop test, not a portable deployment guarantee.

## Review bundles

Each managed run now writes two bundles without removing local artifacts:

- **review-metadata.zip**: results, reports, logs, plans, manifest, error/progress
  snapshots and a `bundle-manifest.json` inventory. Binary artifacts are excluded.
  The bundled report visibly states that images were intentionally omitted.
- **review.zip**: the existing full local evidence bundle, including screenshots.

The terminal prints `REVIEW BUNDLE (NO IMAGES)` for the routine upload. Report and
history links prefer this lightweight bundle; full evidence remains available.
No 7-Zip installation or manual deletion/repacking is needed. These are not privacy
redactions: metadata may include workbook names, cell guard values and local paths.
Independent pixel verification still requires selected original PNGs when requested.

## Timing evidence and WASM decision

The three 2026-10-05 metadata-only runs completed all 11 worksheets. They were
selective-only, not paired A/B comparisons. PNGs were removed before upload, so these
are reported measurements, not independently re-decoded pixel evidence:

| Budget | Final tiles / attempts | Verification groups | Blocker cells | Positive groups | No-change groups | Run wall time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 16 MP | 64 / 64 | 268 | 3904 | 9 | 259 | 489.35 s |
| 32 MP | 37 / 37 | 288 | 3904 | 9 | 279 | 414.26 s |
| 64 MP | 23 / 23 | 273 | 3904 | 9 | 264 | 400.06 s |

Wall time includes launcher/startup/finalization. The previous `analysis_ms` also
includes selective native calls, awaits and persistence; it is **not pure JavaScript
CPU time** and overlaps other timing fields. Do not use it to predict WASM speedup.

This patch adds `safe_capture.compute_profile` with separate `candidate_scan_ms`,
`heuristic_scan_ms`, `pixel_diff_ms` and `roundtrip_compare_ms` for synchronous loops,
and `tile_decode_ms` for image/canvas decode including asynchronous waiting. These
are subsets of existing timing totals, not additional summable phases. Native crop
`finalize_ms` is separate. Disabling gridlines should be measured before porting.

A future Rust pixel kernel can own candidate masks and differential analysis with
shared native/WASM tests, leaving Office.js calls, PNG decoding and report UI in JS.
Benchmark identical buffers and outputs, and include buffer-copy/bridge cost. A
WASM language change alone does not reduce Excel roundtrips, PNG encoding or I/O.
The wasm-bindgen boxed-slice interface copies typed arrays; memory-view designs
require explicit ownership/lifetime handling. No WASM compiler dependency is added.

## Run

1. Use Get picture, F, selective verification, 16 MP and budget-aware row boundaries
   with the ASP.NET host first. Inspect Gridlines, Saved PNG policy and final images.
2. Repeat that configuration with the Rust host and **Excel /x** for the HTTP check.
   A startup failure should still leave a metadata bundle and host/manifest evidence.

No A/B or budget sweep is necessary to verify these plumbing changes. Upload the
`review-metadata.zip` shown by the runner, even after a failure.

## Sources

- Microsoft Worksheet.showGridlines: https://learn.microsoft.com/en-us/javascript/api/excel/excel.worksheet
- Microsoft manifest/transport requirements: https://learn.microsoft.com/en-us/office/dev/add-ins/develop/add-in-manifests
- Office.js performance: https://learn.microsoft.com/en-us/office/dev/add-ins/excel/performance
- wasm-bindgen boxed slices: https://wasm-bindgen.github.io/wasm-bindgen/reference/types/boxed-number-slices.html

## Validation scope

Automated tests cover policy choices, batched view-setting verification, crop receipt
handling, raw-evidence preservation, calibrated buffer planning, report projection,
and existing capture/verification logic. Rust source includes crop/identity tests.
Native Excel, Windows PowerShell/WPF and host compilation must be checked on Windows;
mock or source-contract tests are not proof of native rendering or transport support.
