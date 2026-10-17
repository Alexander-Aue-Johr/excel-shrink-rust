# Contextual capture: final qualification

Status: implemented, native Excel qualification pending. This patch does not select
or replace the production regression engine, modify input files, delete historical
results, or certify a 10,000-file original-versus-shrunk regression.

## One task

**Visual validation: Finalize capture strategy** is available in VS Code Tasks and
Run/Debug. It bridges WSL to Windows PowerShell, preserving the existing Job Object
supervisor, working-copy policy, reports and validated `review-metadata.zip` export.
Run it with the unchanged local corpus and a fixed monitor/scaling configuration.
Do not use the old long decision campaign to test this patch. No external DLL,
certificate, WPF component, Rust crate or Python package is added.

`capture-finalize.json` is the source of truth. It selects all XLSX files recursively
under `corpus-local`; duplicate basenames are isolated by the existing input ID.
It first qualifies common capture readiness. It does not repeat the synthetic
units/rows/plateau suite, historical A–E bounds, VBA/clipboard/GDI+ or shape engines.
Those older results remain evidence of their own versions, and remain accessible.
Failed adapters do not become successful by being excluded from this focused run.

## Why the capture policy changed

Original evidence for the yearly report's `DL-Beschäftigung` compared the lower
edge of `A1:H421` with the upper edge of `A422:H840`. The final PNGs were 799×5865
and 799×5868 pixels. At x=251, the former edge was black and the latter white; the
vertical black line resumed in the second PNG's next pixel row. These are native
outer edges, not a shared worksheet row. A dimension formula with an extra pixel
does not establish that these edges contain interchangeable painted pixels.

The user's neighbour-context explanation motivates the implementation. The patch
does not promote it to a universal Excel API guarantee: a real whole-versus-pieces
comparison must confirm it, and all retained seams remain pixel-exact.

## Three rectangles, never three meanings for one address

1. **Logical core**: nonoverlapping worksheet rows/columns owned by this tile.
2. **Capture rectangle**: core plus visible context at internal tile boundaries.
3. **Retained pixels**: measured crop of the original PNG. The existing right-side
   overflow decision can further reduce its width, without changing its height.

Internal boundaries include at least one adjacent cell with positive visible
extent. Hidden/zero-sized cells do not count. Very narrow/small cells accumulate
until the edge is at least two calibrated pixels inside the capture. Scanning
cannot leave the parent logical range. A zero-only tail is recorded explicitly as
an effectively outer edge. At the whole range's actual outer boundary, no padding
is added: a whole-range reference has that same boundary.

Merged ranges intersecting a core boundary are queried and the capture envelope
includes the intersecting merged area inside the parent extent. This can require
more than one cell. The enlarged envelope is charged to the budget; it is never
silently clipped to fit. ExcelApi 1.13 is required for merged-area inspection. A
failed/unsupported query is not treated as proof that no merge exists. More than
512 merged areas in one query can fail per the API; such a case remains unresolved.

The planner applies both the 30,000-pixel edge budget and the selected 16/32/64-MP
area budget to the **actual padded capture**, before `getImage()`. Full-width
vertical strips are preferred. Geometry guards exist for horizontal cuts too,
but one neighbour does not prove that arbitrary incoming text, a large merge or
a drawing has complete horizontal context. The existing horizontal-layout
inconclusive policy and canonicalizer restriction remain in force.

## Measured geometry, not a guessed Y factor

`capture-geometry.js` maintains one consistent coordinate system: sums of
individual-cell `Range.height` and `Range.width`, with hidden axes contributing
zero. Native `top`/`left` remain diagnostic measurements. XML `ht` and
`RangeFormat.rowHeight` are not substituted for measured cell geometry.

Properties are read in groups of at most 384 cells per synchronization. Prefixes
are cached; tile-boundary binary search then runs without an Office roundtrip at
each midpoint. The needed origin-to-boundary prefix is measured, not worksheet
contents. A hard guard at row index 200,000 prevents an accidental million-row
scan; a larger required extent is explicitly unqualified, never truncated.

Before and after rendering, affected axis entries are refreshed. Changes to
individual heights/visibility remain errors even if their sums would compensate.
A control-to-large-range mismatch, fractional raster phase conflict or exact
context-crop dimension mismatch is not repaired through resampling or tolerance.
Using additive axes on all capture sheets intentionally replaces the earlier
mixed top-difference model; its cost is included in the new timings. Old DPI/source
measurements are timeout hints only, not new timing observations.

## Native crop and exact evidence

The .NET ImageSharp and Rust image endpoints crop x/y/width/height from an already
stored tile. Context removal and right-probe trimming are one rectangle operation.
No second Excel capture, full-sheet canvas or resize is introduced. The immutable
raw PNG, original hash, native range, crop box, output hash and logical offsets
remain linked in the report.

The strict strip digest still checks shared interior pixel rows. No one-pixel
exception, alpha masking, seam deletion, shifting or majority vote is introduced.
The same pixel lattice is shared by adjacent retained cores; the canonical stream
emits duplicate shared pixels once **only after exact equality has been checked**.

## Readiness before ranking

Readiness uses .NET / F / Selective / 16 MP on every screening input (all inputs
when there are at most twelve; the existing size-stratified selection otherwise).
Known yearly-report/height problem files are ordered first.

For each tiled sheet, its first ordinary vertical seam gets a local reference:
two visible rows on either side, retaining the same horizontal probe context.
Four small raw images are persisted: whole range before, padded upper piece,
padded lower piece, whole range again. Each is independently capped at four MP.
The full reconstructed RGBA buffer, shared inner row and repeated whole reference
must agree exactly. A budget failure remains unverified; it is not skipped as a
successful check. Monolithic sheets need no artificial seam test.

This is one **local reference per tiled sheet**, not a full monolithic oracle for
every seam. In addition, all final tile seams pass the strict offline digest.
Repeated original-versus-shrunk testing remains a separate required workflow.

If any common readiness file lacks complete capture or canonical evidence, no
performance matrix starts. Healthy sessions still proceed to other sheets/files,
leaving one concise set of failures. A native timeout or an unconfirmed mutated
working-copy restoration prohibits further Office calls in that session.

After readiness succeeds, the existing bounded decision race compares six cases:

| Host | Overflow | MP |
|---|---|---:|
| Rust | Selective | 16 |
| Rust | Selective | 32 |
| Rust | Selective | 64 |
| .NET | Selective | 16 |
| Rust | Heuristic | 16 |
| .NET | Heuristic | 16 |

The normal race does **not** perform the additional local reference images, so
readiness costs do not bias its performance ranking. Selected-budget interaction
checks, two finalists with repeated same-input runs, real fresh-session 16-MP
recovery costs and the predeclared maintenance preferences remain unchanged.
The rest of a large corpus receives the selected configuration, not every
combination. A provisional selection is never labelled proven correctness.

## Budgets and output

- Session: three hours. Readiness: 45 minutes. Screening: 75 minutes.
- Confirmation/interaction stage: 30 minutes. Coverage uses remaining session time.
- Ordinary case allowance starts at six minutes, adapts to same-input successful
  timings, and is capped at ten minutes. Supervision and postprocessing are also
  bounded by remaining session time. Preparation/final report time is not a hard
  real-time scheduling guarantee.
- No replay of completed native or synthetic historical families in this profile.
- Pending work resumes with unchanged source/config/input identities. A changed
  source revision needs a new campaign; old evidence is never mixed invisibly.

The result is still one history entry, `report.html`, `decision.md`, detailed case
reports, and a validated `review-metadata.zip`. Only that ZIP is needed for review.
It intentionally excludes images but includes their paths, hashes and checks.

## Progress overhead

The complete Rust plan is archived once under `inputs/plans/visual-plan.json`.
The live state stores a compact provenance projection and an artifact reference;
repeated progress snapshots omit the immutable plan. Redundant snapshots remain
coalesced at two-second intervals. Errors, sheet/workbook closure and pending
mutations force persistence. Native starts and saved-image receipts remain
individually acknowledged. This is not a lossy replacement for the error journal
and not a new server-side delta-reconstruction protocol.

## Validation scope

Included pure tests cover hidden/zero/tiny neighbours, merges, non-origin crops,
additive geometry with wrong native positions, changed heights, finite budgets,
exact reference/overlap failure, an integrated modeled F capture, pending-state
recovery and the gating of the decision race. The native crop paths and PowerShell
transport have source-contract tests. A Rust pixel-crop test is included.

Tests against mocks are not an Excel rendering proof. Windows Excel, Windows
PowerShell, .NET/Rust native builds and the native pixel preflight must still be
executed on the user's machine. No generated report in this patch pretends that
those measurements have already occurred.

## Primary API documentation

- https://learn.microsoft.com/en-us/javascript/api/excel/excel.range?view=office-js
- https://learn.microsoft.com/en-us/office/dev/add-ins/excel/performance

The getImage documentation specifies PNG output but no universal neighbour-halo
pixel-equivalence guarantee. Local reference equality is therefore measured.
