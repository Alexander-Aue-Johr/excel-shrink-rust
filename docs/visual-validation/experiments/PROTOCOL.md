# Evaluation protocol

## Questions and blocks

`reproduce.json` fixes input hashes, scopes, variants and deadlines. This is a
bounded engineering evaluation, not a full Cartesian product over every parameter.

| Block | Inputs/scopes | Variable factors | Fixed factors | Repetitions |
|---|---|---|---|---:|
| Coverage | All visible sheets of eight pinned workbooks | Workbook | F/.NET/Selective/32 MP; local context validation | 1 |
| Host/overflow | Hospital wide+overflow sheets; all Förderprogramme; selected Wanderungen geometry sheets | Host (.NET, Rust) × overflow (Heuristic, Selective) | F, 32 MP, identical input scope | 3 |
| Tile budget | Hospital ` KHV_2022`; Kernhaushalt `csv-71717-22` | 16/32/64 MP | F/.NET/Selective | 3 |

The host/overflow block uses 32 MP as a fixed comparison point for the chosen
configuration. The published historical host screen used 16 MP; these distinct
experiments must not be presented as the same data. The budget block separately
isolates image size under .NET/Selective.

The same input scope is a block. Variant order rotates by block and repetition.
The three budget arms each occupy each order position for a given worksheet over
three rounds. The four host/overflow arms with three rounds are not fully balanced.
Fresh instances include startup cost; there is no unrecorded warmup result and no
assumption of steady-state throughput. Cache, load and operating-system differences
remain limitations. Keep display scale and monitor unchanged throughout a run.

## Native and renderer references

The individual interface executes drawing bounds, VBA chart capture, VBA-local GDI+
bitmap/EMF, owned-buffer pair comparison with/without PNG persistence, native row
heights, and synthetic units/markers/row heights/plateau diagnostics. They are not
multiplied through the three default performance blocks. They answer separate
questions and can be reproduced independently when examining those references.

Native capture obtains validated F ranges first rather than inventing a capped
UsedRange as equivalent input. Its raster/tiling policy is nevertheless different.
Clipboard use excludes concurrent clipboard activity. C# Excel-DNA/VSTO outputs are
drawing geometry, not capture-time observations. Unavailable prerequisites remain
unavailable; failed references are not a measured slowdown factor.

## Outcome and admission

Each attempt retains its scope, input hash, effective limits, source fingerprint,
Office/browser/display metadata, workload duration, image receipts, exact range and
canonical decoded-pixel results. A complete process without complete image coverage
is not qualified. Source/input mutation, inconsistent seams, unexpected bounds,
missing PNGs, invalid restoration and result-publication failures remain failures.

Normal tiling includes visible adjacent rows/columns and any required merged context
inside both the edge and area budget. Hidden/zero-size neighbors do not establish
context. Final cores are cropped without resampling. Horizontal tiling not supported
by the canonical strip validator remains unverified, not silently stitched.

Coverage performs a local whole-versus-two-contextual-parts test on the first seam
of each tiled worksheet, plus a repeated whole reference. This is not a monolithic
oracle for every seam of every worksheet. All final internal overlaps are checked
strictly by the canonical digest. Complete repeated output must agree across
variant comparisons under compatible recorded environments.

## Timing and decision rule

Workload includes preparation, Excel startup, capture, analysis and final export.
Offline pixel digest work and report creation are excluded. Diagnostic context
reference captures occur in coverage, not in the performance ranking. Phase timers
may overlap and must not be added as independent components.

Rank the sum of per-scope median workloads only for complete, repeatable,
quality-compatible observations. Keep failed/time-limited trials in the ledger.
A timeout is censored, not a completed duration or a free failure. The tile-budget
policy can retry a failed/unverified >16-MP attempt once in a fresh 16-MP session,
retaining exact host, sheets and method. Charge the failed attempt including waiting
and teardown plus the fresh recovery workload. An actual pixel disagreement remains
visible; a later success is not permission to suppress it.

Preferences are declared engineering policy, not significance tests: Rust may be
preferred within 15% for maintenance; Selective within 15% for explicit intervention
evidence; the smaller image budget within 5% of the fastest eligible sum. Preferences
do not compound without bound. A unique successful survivor alone is not an observed
performance win over unavailable candidates.

## Reproduction boundary

Recorded canonical equality concerns unchanged inputs. Before a production
original/shrunk test, confirm equal unmodified copies, detected deliberate visible
changes (including tiled data), equal comparison scope/environment, and fail-closed
handling of missing images/timeouts. A small engineering corpus is not a reliability
certificate for 10,000 arbitrary spreadsheets.
