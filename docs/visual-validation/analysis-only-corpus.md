# Selected-range corpus analysis

The `--preparation-hash-profile default` workflow now ends with its analysis.
It does not schedule the old full visual comparison, including when analysis
fails or finds different hashes. The option name remains compatible with
existing launch configurations. Other explicitly selected legacy CLI strategies
are unchanged.

## Execution and evidence

1. Prepare the selected shrink profile.
2. Open the original and prepared workbook once, plan the common ranges, and
   apply the existing tall-sheet selection policy.
3. Capture those ranges, compare hashes, and retrieve verified mismatch images
   before recalculating either workbook.
4. Recalculate, capture the same selected ranges again, and retain a separate
   comparison. This does not replace the raw-input comparison.
5. Close the owned sessions and publish the final workbook result.

`preparation_hash_screen.raw_visual` and `.recalculated_visual` retain the two
results. `stable` states whether the two sweeps agreed. It is not permission to
discard a raw-input difference. A failed operation remains `skipped` with its
error; missing partner evidence never means equality. `screened-equal` refers
only to the analyzed raw-input ranges. Omitted areas and the content beyond the
fixed 1,920-pixel right margin remain unchecked. Chartsheets are outside the
worksheet capture planner's scope; tolerating their presence does not validate
their appearance.

The capture policy identifier changed to `selected-range-analysis-v2`. Old hash
caches cannot satisfy it. A cache is reusable only with a completed analysis in
the workbook record; a crash between cache publication and the record checkpoint
therefore causes a fresh attempt rather than silently losing the result.

## Manual inspection

The Observatory's **Capture full sheet** control starts an independent
Original/Default inspection. Its numeric field is the one-based workbook sheet
index, including positions occupied by hidden sheets and chartsheets. Only a
visible worksheet accepted by the capture planner can be requested.

It captures all planned tiles for that sheet, including equal tiles, rather
than applying the tall-sheet sampling policy. It uses Office.js, disposable
input copies, and the current hidden-window setting. Image limits still apply.
Only one extra manual inspection can run per Observatory host. Its images,
completion/error status, and links from the parent group survive page reloads.
**Back to analysis** returns to the original result. Manual inspection never
changes that result or the corpus queue.

The CLI equivalent is:

```powershell
excel-shrink-cli.exe regression capture-sheet `
  --original original.xlsx --variant default.xlsx --sheet 1 --output new-directory
```

## Failure investigation, 1 November 2026

The archived run contained 985 records with a stored `hash_sweep_error`. These
are historical failure records, not a list of operations the current resident
code necessarily still executes. Their later fallback outcomes are separate.

| Stored check failure | Files | Findings and disposition |
| --- | ---: | --- |
| Right edge not verified | 317 | Old resident error wording. Failure to establish overflow safety, not proof of a shrink difference. Current resident capture uses a fixed right margin. |
| Image dimensions/budget | 175 | Oversized full-width/calibration images; one example requested 38,712 pixels across. No unsafe enlargement of the budget. These need individually bounded geometry investigation. |
| Non-worksheet member | 121 | Every inspected package contains a chartsheet. The tall-sheet metadata reader incorrectly rejected the whole workbook, although the capture planner already selects only worksheets. Fixed by skipping non-worksheet descriptors while preserving workbook indexes. |
| Image changed after overflow restoration | 95 | Old temporary blocker edits were not pixel-stable after restoration. The records do not establish why. This probe is absent from the new resident analysis path. |
| Protected sheet | 62 | Stored errors explicitly say protection prevented the old text-overflow probe. No protection is removed to force a pass. |
| Drawing outside capture range | 50 | Reported shape bounds exceeded the planned extent. Remains unresolved; accepting clipped captures would conceal possible differences. |
| Drawing bounds unresolved | 47 | Package evidence requested shape verification but usable bounds were unavailable. One inspected example has only `legacyDrawingHF`, a header/footer object. This needs separate handling rather than assuming every object is a normal worksheet shape. |
| Unsafe overflow probe | 30 | Old intervention could not establish safe conditions. No new probe is introduced. |
| Overflow orientation unavailable | 20 | Old probe could not establish text orientation. |
| Add-in readiness timeout | 18 | No ready signal within the deadline. Stored messages alone do not distinguish blocked Excel, taskpane startup failure, or another initialization delay. |
| Probe changed unrelated pixels | 10 | Old intervention affected pixels outside its expected area. |
| Image calibration inconsistent | 8 | No safe/consistent point-to-pixel calibration. Retained as unresolved. |
| System memory/commit reserve | 8 | Some records had ample physical RAM but insufficient system commit headroom. The reserve is not simply a physical-RAM check. |
| Excel operation timeout | 7 | Timed-out operations or unresponsive Excel; no unique deeper cause established. |
| Existing taskpane | 4 | Injection deliberately refuses existing `xl/webextensions/taskpanes.xml`. Preserving and merging existing add-ins needs its own implementation; no overwrite is attempted. |
| Registry enumeration | 3 | `GetValueNames` failed with “No more data is available.” Registration now reads its known GUID directly instead of enumerating a concurrently changing key. |
| Permission denied | 2 | Office API permission failures; precise restriction not established. |
| No visible remaining columns | 2 | Geometry could not find a usable visible continuation. |
| Worksheet geometry changed | 2 | Old overflow verification observed changing geometry. |
| Office API error | 2 | Internal/missing-resource errors, without enough evidence for a narrower cause. |
| Missing sheet relationship | 1 | Workbook mapping could not be resolved. Not silently accepted. |
| Unsupported column width | 1 | Width 291.7109375 was rejected. No speculative geometry reinterpretation. |

The complete filename/error list is in the dated backup's
`failure-investigation/files.csv` and `files.json`. That directory also contains
the earlier searchable HTML investigation. Its old live download links require
the corresponding archived host/data; the backup itself retains the bundles.

## Verification

- Focused tests cover terminal failed analysis, incomplete results, raw versus
  recalculated outcomes, cache invalidation, and stable worksheet indexes beside
  chartsheets.
- A real Excel fixture with a deliberate visible text change produced mismatch
  evidence before and after recalculation within one session. A font-only fixture
  rendered identically in this Office.js path and was not used as mismatch proof.
- An isolated corpus with a small real Data.gov workbook and a deliberately
  invalid workbook finished with `screened-equal` and `skipped`, respectively.
  Both final executions were preparation/analysis jobs; no legacy comparison
  was scheduled.
- The real manual-capture HTTP endpoint captured one sheet and returned both
  Original and Default images with a terminal completion status.
- The direct registry read was checked with a fresh test GUID, an existing value,
  restoration, and removal of only the test value.

This is focused validation, not a claim that all previously failing workbooks
are repaired. The restarted corpus is deliberately a new measurement run.
