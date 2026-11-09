# Destatis web preparation acceptance on 2026-11-09

## Frozen launch

The optimized `f099d740` runtime contained both executables and 107 supporting
assets. The package verifier checked all 109 files. The originals directory
contained all 1,182 pinned Destatis XLSX members, verified against the embedded
profile before admission. No Check Performance process was started by the prior
originals acquisition or selection checks.

The preparation was started through Edge's ordinary configuration form, using
Prepare Check Performance variants, XLSX directory, Destatis and 16 workers.
The submitted job began at `2026-11-09T01:51:11Z`. A subsequent read-only process
inventory found 16 Excel processes. The host recorded 1,182 selected inputs,
16 in-flight requests and 1,166 queued inputs. This verifies this machine's web
start and initial native admission, without establishing corpus completion.

The frozen source manifest SHA-256 was
`95ca9fd4a2e3a720eed42936222aea64e31e67fbdd30a127ddfaf7642536ad0b`.
The first 16 inputs obtained confirmed native optimization outcomes and retained
the original-saved, before-optimization and after-optimization controls.

## Warm-session defect

Follow-up requests exposed `Previous workbook performance pane did not close`.
The adapter used `ExecuteMso('CheckPerformance')` as a close operation. This
operation did not reliably close the previous pane. Failed requests retained
their own failure receipts, and the pool recycled those sessions.

A separate three-request test repeated the same unchanged original,
`01011970_Auszug_GV.xlsx`, in one Excel process. Its baseline SHA-256 was
`9875a864482bba19e1b480452158b42283239931bd7d48d688308fd93894622c`.
The first request reported confirmed optimization, the second reported no work,
and the third failed while resetting the pane. An XML check found identical
sheet members in both before-optimization controls, containing 315,774 cell
records. The first after-optimization control contained 315,733 records. The
second retained all 315,774 records and the unchanged sheet XML hash.

This is evidence of an ineffective repeated optimization with stale no-work
evidence. A native no-work message in a reused session is consequently
insufficient acceptance for this adapter version. Preserve these observations
as development evidence and exclude unverified warm-session outputs from final
benchmark conclusions.

The stronger repetition test rejected three proposed warm resets. The attempts
used an explicit native Close action, command-bar visibility reset and distinct
temporary workbook names. These attempts remain separate private scratch runs.
They did not establish reliable native reset. Their changes were removed.

## Fresh-session correction

The continuously refilled pool now withdraws a successful Check Performance
session before publishing its response and replaces Excel for the next input.
The conversion pool retains its existing persistent-session policy. Native
receipts state `fresh-excel-per-input`, and the changed worker recipe prevents
silently reusing the earlier preparation generation.

The revised native regression test processed the same original three times in
three different Excel PIDs. All three reported confirmed optimization, retained
the three controls and produced identical after-optimization sheet XML hashes,
`d7fb666135aa4bfc22b425f827e80f729bdacb8213668b4cac3e569b4f84caa6`.
All three after controls contained 315,733 cell records. The final progress
receipt reported 110.68 seconds, three prepared inputs, zero failures and zero
unresolved inputs. Owned Excel process exit was confirmed. This single run
occurred alongside the 16-worker development attempt and is functional evidence,
with no controlled throughput claim.

## Bounded pane-query optimization

The adapter's repeated raw-tree walks inspected the entire Excel window before
inspecting the relevant pane, including a second full-tree diagnostic walk.
The revised adapter locates native `MsoWorkPane` elements through one scoped
UI Automation property query and walks only the selected pane's controls. It
preserves the unique-pane, unique-action and positive-completion requirements.

The same three-request positive fixture passed after this change, using three
fresh Excel PIDs with identical after-optimization sheet XML to the previous
test. Its progress receipt recorded 38.66 seconds. Individual optimization
phases took 5.06, 5.24 and 5.04 seconds, compared with 33.04, 34.97 and 22.84
seconds in the prior functional run. These sequential exploratory runs had
concurrent development load and do not estimate a corpus-wide speedup.

A second three-request test used the already optimized control as its unchanged
source. All three fresh Excel processes reported `no_optimization_needed`, with
zero failures and unresolved inputs, in 25.69 seconds. Owned process exit was
confirmed by both native tests. The private receipts are
`cp-fresh-native-acceptance.json` and `cp-pane-query-acceptance.json` beside the
scratch test roots. Full 16-worker corrected-corpus acceptance remains open.

## Portable release web start

The optimized binaries built from `121a4622` were packaged with the generic
starter and matching runtime assets at `d78e9957`. All 111 package members
passed integrity verification before and after the test. The packaged starter
was launched with Cargo absent from PATH and its inherited runtime overrides
cleared. It selected adjacent executables and capture assets and placed default
downloads, temporary files and logs in the sibling `deltasieve-data` directory.
The first idle host was inspected and closed before the native UI test.

Through the ordinary web configuration, three identical source copies with
distinct names were selected as a custom directory and processed with target
one. The release job completed all three in 57.28 seconds, with confirmed
optimization, distinct Excel PIDs and `fresh-excel-per-input` measurements.
The pool confirmed stopped before the completed test host was closed. The
1,182-input development host was kept separate throughout this acceptance.
This test establishes the local packaged web-to-native path for the bounded
fixture. Other-machine and full corrected-corpus acceptance remain open.

For the already optimized fixture, every before/after control pair differed
only in the modification timestamp in `docProps/core.xml`. Its receipt is
`cp-no-work-query-acceptance.json`. The packaged web job and startup paths are
retained under `portable-starter-acceptance-20261109` in private evidence.

The corpus remains an incomplete acceptance attempt until the defect and its
affected outcomes have been resolved. Later attempts must retain the prior
generation, failures and attribution rather than rewriting their receipts.

## Visible-window countercheck

The receiving-machine follow-up opened two byte-identical original copies of
`01011970_Auszug_GV.xlsx` through the visible Excel interface. The first copy was
opened in a new instance. The second was opened with Excel's Open dialog while
the first remained open. Review > Check Performance and Optimize all were clicked
in each workbook window. Both initially reported 63 optimizable cells and then
71 optimized cells. After saving, both had 315,733 worksheet cell records and the
same concatenated worksheet XML SHA-256,
`d7fb666135aa4bfc22b425f827e80f729bdacb8213668b4cac3e569b4f84caa6`.
The input copies had identical SHA-256
`9875a864482bba19e1b480452158b42283239931bd7d48d688308fd93894622c`.

This countercheck demonstrates successful reuse in the visible-window workflow.
The earlier automated failure does not establish that Excel process reuse itself
causes stale results. A third unchanged copy was opened after closing the saved
second workbook, keeping the first workbook open in the same instance. It again
reported 63 optimizable cells and 71 optimized cells. This narrows the diagnosis
further, although the retained workbook was an optimized original instead of the
worker's blank settings workbook. The private desktop, visibility transitions,
SaveAs controls, command routing and pane observation still differ.
Fresh-per-input retirement remains a verified workaround pending diagnosis.

## Requested order and process-restart matrix

A is `01011970_Auszug_GV.xlsx`, with original SHA-256
`9875a864482bba19e1b480452158b42283239931bd7d48d688308fd93894622c`.
B is `01011975_Auszug_GV.xlsx`, with original SHA-256
`79e6c3a844974e6daa005df47b24ae6310aa08e0a42ea5603e078899583bc3ef`.
Each case received new original copies. The private test used the retained warm
worker program, the current preparation-control module and the bounded pane-query
module, with identical asset hashes across cases. The native desktop and worker
settings were retained. This tests the automated worker lifecycle, separately
from the visible manual-interface countercheck above.

| Lifecycle | Order | A worksheet cells before / after | B worksheet cells before / after |
| --- | --- | --- | --- |
| Reused Excel, first run | A then B | 315,774 / 315,733 | 115,777 / 115,777 |
| Reused Excel, first run | B then A | 315,774 / 315,733 | 115,777 / 115,699 |
| Reused Excel, repeat | A then B | 315,774 / 315,733 | 115,777 / 115,777 |
| Reused Excel, repeat | B then A | 315,774 / 315,774 | 115,777 / 115,699 |
| Fresh Excel per file | A then B | 315,774 / 315,733 | 115,777 / 115,699 |
| Fresh Excel per file | B then A | 315,774 / 315,733 | 115,777 / 115,699 |

All before-optimization worksheet XML hashes matched within each input identity.
The three unchanged second-position outputs corresponded to native no-work
messages. Successful outputs for A shared worksheet XML SHA-256
`d7fb666135aa4bfc22b425f827e80f729bdacb8213668b4cac3e569b4f84caa6`.
Successful B outputs shared
`9b7834a864a3fd145c327a0422d9ad60c03186212b2a9c40082889130adc81af`.
The fresh cases had four distinct Excel PIDs. Each warm case kept one PID across
its two requests. Receipts, source copies, asset hashes and independent worksheet
checks are retained in the private `cp-order-matrix-20261109` evidence directory.

The repeated reverse-order results differ, so a deterministic file-order rule
is unsupported. The automated reuse policy remains unreliable on these inputs.
Fresh-per-input workers passed all four matrix requests and remain the production
workaround. These small tests establish functional evidence without estimating
corpus throughput or the exact internal cause of the native state failure.
