# Visible controls and focused capture repair

## Evidence and scope

The prior decision run stopped on Kernhaushalt worksheet `71717-50` with
`CALIBRATION_NO_SAFE_CONTROLS` (`candidate_count: 4`). Its archived Rust plan marks
column A as hidden. Columns C and D have unusually large stored XML widths.
The old control generator anchored every candidate at the same top-left cell.
It discarded zero-width/zero-height and bootstrap-budget violations, then required
three distinct widths AND three distinct heights in the same three rectangles.
A hidden first column was therefore filtered, but the remaining insufficient pool
was not recovered. The missing per-candidate geometry prevents attributing every
rejection in that old run retrospectively.

This change does not unhide/widen columns, adjust rows, activate worksheets, alter
styles or change the logical Rust bounds. The input workbook remains untouched.
Stored XML column widths are not point measurements. New diagnostic columns use
Office.js `columnHidden`, `left` and `width`; rows report both A/next-column anchors.

## Control selection

1. Try the existing small origin controls with the existing fit and holdout.
2. If no suitable set exists, try two bounded sets of offset windows. Each contains
   at most 54 rectangles. No whole-worksheet content scan is performed.
3. If a joint triple is still unavailable, span X and Y independently with up to six
   fitting controls and one holdout/repeat. All fitting observations still need to
   satisfy the same scale model; axes are not assumed equal.
4. If the model is inconsistent, retain the failure. No borrowed factor, forced
   96-DPI conversion, tolerance expansion or arbitrary best-looking fit is used.

Control records explain zero extent, subpixel bootstrap extent, edge budget, area
budget and duplicate visible extent exclusions. A fallback control search triggers
one small read-only geometry audit. Entirely hidden regions never force a zero-size
image request. The search is finite; it can still return a worksheet failure.

## Worksheet isolation

Known read-only calibration/geometry failures are local to that worksheet. The
worksheet remains `failed`, its data and PNG receipts remain, and a read-only native
health call must finish within ten seconds before moving to the next worksheet.

Fatal transport/persistence errors, a dead native session, and an unconfirmed
mutation are not made recoverable. A native timeout does not cancel the native
work: the session is stopped and the existing process supervisor remains in charge.
A good later sheet never upgrades an earlier failed sheet to success.

Recovery after a normal image failure uses an already validated visible calibration
range rather than hard-coded A1, which itself may be hidden.

## Progress and geometry evidence

Native operation starts and host image receipts are still individually acknowledged
and journalled. Only redundant full-session progress snapshots are coalesced with a
two-second minimum interval; errors, worksheet boundaries and workbook completion
force a snapshot. A hard stop can leave the latest coalesced state behind its host
journal/PNG receipts. No completed image file is deleted.

`checkpoint_ms` exposes full-state snapshot cost separately; named phase timers can
overlap and must not be added without checking their scope. Runtime changes caused
by this reporting policy must not be presented as a host/MP algorithm speedup.

The worksheet report exposes **Calibration controls**, **Hidden / narrow axes and
geometry**, and **Worksheet failure isolation and session health**. When geometry
fails, a bounded row/column audit records actual dimensions and hidden flags, not
cell values. Selected worksheets are identified explicitly; unselected worksheets
are labelled `not-selected`, not failed or successfully covered.

The canonical seam validator remains strict. An inconsistent overlap now includes
zero-based row, first/last differing X, changed-pixel count, source tile paths and
BGRA bytes at the first difference. No border pixel is silently discarded, shifted,
scaled or ignored. This is diagnosis, not a seam correction.

## One bounded follow-up task

Run **Visual validation: Check capture fixes (focused)**. The task bridges WSL to
Windows PowerShell and shares the existing controller, process ownership, deadlines,
source hashing, report history and verified metadata archive. Run/Debug also has an
entry with that name.

`capture-followup.json` deliberately does not change `decision-campaign.json`.
The focused run uses .NET / Strategy F / Selective / 16 MP as a fixed debugging
configuration, not as a final method or image-budget recommendation.

| Workbook | Worksheets / question |
| --- | --- |
| Kernhaushalt | `71717-50`, `71717-51`: hidden/wide controls and continuation |
| Stromsteuer | `Erläuterung zu CSV-Tabellen `, `csv-79932-01`: explanatory sheet and continuation; trailing space is significant |
| Wanderungen | `Titelseite`, `Inhalt`: unresolved Y geometry and continuation |
| Verkehrsunfälle | `1.1`, `1.2`: PNG/point disagreement and continuation |
| Jahresbericht | Four previously affected employment/turnover sheets: strict seam evidence |
| Förderprogramme | One all-visible-sheet pass: progress snapshot overhead and bounded completion |

At most six workbook cases, with no host/MP matrix or repeated synthetic/native
reference suites. The session budget is 30 minutes; each capture case is limited
to at most six minutes, with up to 90 seconds for postprocessing subject to the
remaining session budget. Cleanup may extend the terminal shutdown briefly.
Pending cases stay pending and can resume. Native timeouts remain censored failures.
The exact per-case timeout appears in the report.

A full strategy decision and the 10,000-file original-versus-shrunk regression run
remain separate. Existing experiments and prior failures are retained, not deleted.
After these repairs are checked, use the prior budget/host evidence for a small
remaining comparison rather than restarting every diagnostic.

## Deliverable and validation

The standard `review-metadata.zip` contains results, logs, search audits, scope,
strict seam diagnostics and linked reports, but no PNG bytes. Submit this one
archive from the focused run. It is published only after archive validation.

New tests cover hidden/narrow prefixes, broad adjacent columns, independent-axis
controls, finite failure, continuation of a second sheet, strict fatal-session
handling, exact whitespace-sensitive selection, snapshot coalescing and mocked
focused controller/report/archive execution. They are protocol/model tests, not
native Excel renderer tests. One pre-existing source-contract assertion was updated
to accept both existing metadata-banner phrasings (`bundle`/`copy`); runtime archive
behaviour and the image-exclusion assertion are unchanged.

Windows PowerShell, Excel, COM and C# native execution must be checked on Windows.
No successful native Kernhaushalt capture is claimed before that run.
