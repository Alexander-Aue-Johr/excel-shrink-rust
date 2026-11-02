# Targeted pipeline optimization checks — 2026-11-02

These are operational checks, not an isolated benchmark for the paper. Native
tests use owned private desktops and verify Excel exit. No capture coverage was
removed. The Data.gov subset and restored source files remain local artifacts.

## Serial versus parallel captures

One deterministic workbook, four states, 33 selected regions per state, release
build `b784fa5`. Both runs produced identical pixel/geometry verdicts and capture
counts. One run per mode is insufficient for a precise cross-machine speed claim.

| Mode | Capture selection/transport | Staged image comparisons | Whole workflow |
|---|---:|---:|---:|
| Serial | 9.095 s | 0.430 s | 65.78 s |
| Parallel | 8.268 s | 0.460 s | 63.71 s |

Both read 132 immutable images and decoded only 65 distinct encoded contents.
The native bridge test separately confirms all lane requests are submitted before
any response is required, and reverse-order replies retain their correct identity.

For reproducible small protocol checks:

```powershell
cargo build -p excel-shrink-cli
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/targeted-comparison/test-batch-bridge.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File docs/visual-validation/experiments/targeted-comparison/run-smoke.ps1 -Output .tmp/parallel-recalc-check -RecalcCandidate
```

Use a new output directory for each run. Add `-SerialCaptures` to the smoke runner
or private-desktop `run-targeted-comparison.ps1` launcher for a serial diagnostic.
Existing corpus commands need no additional arguments.

## Real large-report case

`EPA Phase II Cookstove LCA Results - GH_fbb1dbc6645b.xlsx`, Original SHA-256:
`fbb1dbc6645b85cbe3f8397282409bfbff35e10678bb95689c897ffe4b539388`.
Original and Default were restored from the verified existing preparation bundle.

The binary/parallel/compact-audit iteration completed all six states with 169 cell
regions each. It produced a 9,139,208-byte main report instead of the earlier
1,587,484,832-byte report. All 3,312,863 XML changes remained in gzip sidecars
totaling 35,133,538 bytes. Pixel comparison statistics: 4,637 requests, 1,296 image
files read, 292 distinct contents decoded, 1,004 encoded-image reuses. Staged image
comparisons took 2.563 seconds; capture selection/transport took 80.193 seconds.
The full native workflow took 587.9 seconds, including XML work and Excel startup.
Other private-desktop smoke tests overlapped, so this is not a controlled wall-time
comparison with the earlier five-group run.

This measured iteration preceded the subsequent immutable XML snapshot cache and
further reduction of raw candidate lists in final JSON; it does not claim their
additional speedup. A later native Check Performance smoke run reused 29 of 38 XML
reads and decoded one image for 23 comparisons across six files.

Additional real Excel checks passed: late formula candidates and Original-last
ordering; unchanged/changed/added/deleted/zero-area charts; native Check Performance
removal of an exterior formatting tail; and an injected late capture failure that
remained failed and closed its own processes while the independent EPA group kept
working. Unit tests round-trip all 100,000 audit changes and verify that differing
PNG encodings with identical pixels remain equal, invalid/missing evidence fails,
and compressed audits accompany verified result export/import.
