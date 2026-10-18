# Capture-method decision

Status: **selected engineering configuration**, based on the linked measurements.
The replicated image-budget comparison supports 32 MP. Full corpus capture qualification
remains open for four worksheet outcomes listed below; method selection is not a
production-regression certificate.

## Configuration

| Component | Selected policy |
|---|---|
| Bounds | Rust OOXML metadata plan, Strategy F, supplemented by validated drawing geometry |
| Native rendering | Excel Office.js `Range.getImage()` |
| Local host | .NET; the rest of Excel Shrink remains Rust |
| Overflow | Selective whitespace intervention with exact restoration and pixel comparison |
| Gridlines | Disabled for both members of a regression pair; explicit cell borders remain |
| Raw image budget | 32,000,000 pixels and 30,000 pixels per edge, simultaneously |
| Internal tile boundaries | Visible neighbouring worksheet context; crop the interior without resampling |
| Geometry | Measured additive individual row heights/column widths; bounded fixed-span handling |
| Pixel validation | Canonical decoded pixels; no scaling or tolerance |
| Recovery | Bounded, fresh 16-MP attempt where supported; preserve primary failure and cost |

The machine-readable settings are in [capture-policy.json](capture-policy.json).
A recovery must not erase a pixel mismatch, input change or failed restoration.
The native call, startup, capture and session deadlines remain finite. Incomplete
captures and failed result publication are never interpreted as image equivalence.

## Evidence and choice

The [published record](evidence/20261017-reviewed-captures/index.html) contains the
relevant individual observations and source/run hashes. Its groups originate from
different runs and are not pooled into a single factorial experiment.

**Host.** In the preceding Selective/16-MP screening, five complete, equal-output
workbook pairs gave a median Rust/.NET workload ratio of approximately **3.34**.
This evaluates the implemented hosts including their communication and telemetry,
not an intrinsic performance claim about Rust versus C#. The gap exceeded the
predeclared 15% maintenance preference for Rust. .NET was selected as the local
capture companion rather than opening another optimization project.

**Overflow.** Six complete .NET screening pairs gave a median Selective/Heuristic
workload ratio of approximately **1.089**. Selective provides an explicit blocker
intervention and confirmed restoration on candidate rows. The overhead was accepted
within the declared 15% preference for this additional evidence. It is not a proof
that the method handles every possible Excel drawing or layout.

**Image budget.** The final controlled .NET/Selective experiment ran two selected
worksheets × three budgets × three repetitions. All 18 cases completed, all 549
final tiles had local pixel digests, and all 531 checked seam rows agreed. The
canonical image of each worksheet agreed across all nine corresponding runs.

| Budget | Hospital median (s) | Long-table median (s) | Sum of medians (s) | Relative to fastest |
|---|---:|---:|---:|---:|
| 16 MP | 92.29 | 162.17 | 254.45 | +9.52% |
| **32 MP** | **83.33** | **156.60** | **239.93** | **+3.27%** |
| 64 MP | 77.20 | 155.15 | 232.34 | fastest |

The rule, fixed before inspecting the final result, selects the smallest qualified
budget within **5% of the fastest sum of per-worksheet workload medians**. Therefore
32 MP is selected. **64 MP was fastest and had no observed failures in this test.**
It was not rejected as unreliable. The lower raw-image budget is not a measurement
of halved process memory. Workload includes setup, fresh Excel startup, capture,
analysis and export, excluding offline digest computation and report generation.
For a different long-lived production process, the balance may differ.

The final budget source run is `20261017T173952378Z-614e6f83`, source fingerprint
`9b91096b3bc97fed84ea75c5724d0d327df66f10c93511f860ce6449cd45a003`.

## Current protocol replication

The [2026-10-18 execution record](evidence/20261017T235702925Z-16893556/index.html)
contains all 62 observations of `reproduce.json`: **48 complete and 14 failed cases**,
with status `complete-with-gaps`. All planned cases ran. Twelve failures repeat the
same two Wanderungen drawing-bound outcomes in the host/overflow block; the two
coverage failures contain those two outcomes plus two Verkehrsunfälle outcomes.
They are not 14 independent defects and are not reported as renderer crashes.

The 18 .NET/Selective budget observations all completed without recovery. For each
of the two worksheet scopes, canonical pixels agree over all nine observations.

| Budget | Hospital median (s) | Long-table median (s) | Sum of medians (s) | Relative to fastest |
|---|---:|---:|---:|---:|
| 16 MP | 87.54 | 157.60 | 245.14 | +6.31% |
| **32 MP** | **81.41** | **154.29** | **235.70** | **+2.22%** |
| 64 MP | 78.31 | 152.28 | 230.59 | fastest |

The same predeclared 5% rule selects **32 MP** again. This replication does not
replace or pool the earlier observations. The host/overflow comparison has an
incomplete geometry block, so it does **not** establish a new overall host ranking.

Coverage with .NET/Selective/32 MP completed six of eight workbooks, including all
**107 Kernhaushalt worksheets**. Across the coverage cases, 462 of 466 requested
worksheets were captured. The remaining outcomes are:

| Workbook | Worksheet (original index) | Outcome |
|---|---|---|
| Verkehrsunfälle | `2_(1)` (7), `2_(2)` (8) | `DRAWING_BOUND_UNRESOLVED` |
| Wanderungen | `Vorbemerkung` (3), `Anhang` (35) | `DRAWING_BOUND_UNRESOLVED` |

The observed shape targets extend beyond the final boundary reported by the native
grid probes. The bounded mapper refuses an unchecked last-row result. These four
outcomes remain unresolved; no drawing is discarded, no pixel tolerance increased,
and no smaller range is declared complete to close the gate. `Qualitätsbericht`
is not one of the failing worksheets in this replication.

Run `20261017T235702925Z-16893556` records source fingerprint
`edb9085f4f7460c402790ad857157ef5efd4e266baffc562c6b447738b4536e0`.
The linked record preserves input hashes, settings, case statuses and pixel hashes.
Post-run documentation and reporting changes are not presented as another native
execution of the capture methods.

## Alternatives and scope

VBA `CopyPicture` uses the shared system clipboard. Existing native screenshot
references have their own coverage/raster rules and some failed to complete; they
do not support a fair universal “VBA is X times slower” statement. Clipboard
corruption is not asserted without specific evidence. Excel-DNA/VSTO benchmarks in
this repository measure **drawing bounds**, not an independent C# screenshot engine.
These reference methods remain directly runnable through the individual-experiment GUI.

Visible-context capture resolves the tested outer-edge discrepancies, and the
additive and fixed-span geometry modes handle the tested height and single-column
cases. These are bounded empirical findings, not universal Excel invariance claims.
The eight-workbook screening corpus cannot establish reliability on 10,000 arbitrary
files. The actual original/shrunk regression remains a separate acceptance and
measurement task.

## Reproduction and updates

Run [reproduce.ps1](experiments/reproduce.ps1) using the pinned
[configuration](experiments/reproduce.json). Inspect the new local report and then
explicitly [publish the compact evidence](experiments/REPORTING.md). Published
records preserve failures and source identities. New measurements do not silently
rewrite this decision; amend the table and rationale only after reviewing the new
scope, environment, repeats and quality conditions.
