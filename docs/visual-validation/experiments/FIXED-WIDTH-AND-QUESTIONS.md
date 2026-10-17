# Fixed-width capture and remaining question profiles

## Scope

This change closes a specific calibration limitation without weakening the normal
two-axis model, changing workbook contents, or declaring a production winner.
The focused runner, finite phase timeouts, owned-process cleanup, image receipts,
visible-cell context, exact interior crops, strict canonical pixel checks and
metadata-only review archive remain the execution path.

The prior focused evidence showed successful small full-width captures on
`Vorbemerkung`, `Anhang` and `Qualitätsbericht` in
`wanderungen-2010120217005.xlsx`. Suitable controls differed in height but not in
visible width. Those observations motivate the implementation; they are not
substituted for a new run or used to hard-code a DPI factor.

## Fixed horizontal interval

The existing origin, offset and independent-axis searches run first. When only
X variation is insufficient, the fallback must find at least three safe controls
with different heights over the requested complete physical horizontal interval.
It requests the exact same columns for each control. A fourth control is an
independent height where available, or a repeat otherwise.

* Y is fitted and validated using the existing height model.
* X is recorded as `mode: fixed-span`, with `scale: null`, physical left/right
  bounds, exact column interval and the repeatedly observed PNG width.
* A different width in the fitting controls, holdout or subsequent image fails.
  The fixed-width holdout requires exact integer dimensions. Normal two-axis
  tolerances have not been increased.
* A shifted or changed horizontal interval, horizontal tiling, horizontal context
  crop or speculative horizontal overflow buffer is not supported by this mode.
  It is rejected, not silently measured using Y's scale or a neighbouring sheet.
* Vertical tiling and visible-row context are allowed. Every tile keeps the full
  calibrated width; the existing host crop may remove vertical context only.
* Hidden flags alone do not determine size. A measured zero-width column does not
  create a new calibration width, even when its reported hidden flag is false.

A constant width cannot independently identify an arbitrary horizontal scale and
image intercept. This fallback does not claim to do so. The entire horizontal
span is kept; there is no resampling or arbitrary horizontal coordinate mapping.
The output can still be compared by the ordinary exact decoded pixel digest.

Relevant errors include `FIXED_WIDTH_PNG_MISMATCH`, `FIXED_WIDTH_SPAN_CHANGED`
and `FIXED_WIDTH_HORIZONTAL_OPERATION`. A missing safe Y span or an oversized
single-row capture remains a failure. The old diagnostic-only control pictures
remain available when the bounded fallback cannot qualify the requested interval.

No new browser module is added. Both hosts use the same existing JavaScript files
and their existing routes and asset preflight.

## One focused entry point

Use the existing `Visual validation: Check capture fixes (focused)` task, or:

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -InterfaceMode gui
```

The GUI adds a **Question profile** selector. Choose **Custom / named issue cases**
for the previous file/sheet controls. For a profile, the predefined exact scopes
are displayed, fields varied by that profile are disabled, and the other capture
settings and finite phase budgets remain editable. The GUI saves the request in
`.tmp/visual-validation/focus-last-request.json` as before.

The command-line alias `-Profile` maps to `-QuestionProfile` (it does not replace
PowerShell's automatic `$PROFILE` variable). A profile cannot be combined with
`-CaseId`, `-Workbook`, `-Sheet`, `-SheetIndex`, or explicit request targets.
Use one selection method. The corpus directory may still be overridden.

## First run: only the three remaining Wanderungen sheets

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Profile fixed-width -PlanOnly

& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Profile fixed-width
```

This opens the workbook twice in independent cases and captures only the three
specified sheets. It checks completion, canonical pixel evidence and unchanged
output between repetitions. It does not reopen Kernhaushalt's preceding 80 sheets
or trigger the finalization matrix. Default capture allowance is 300 seconds per
case, startup 300 seconds and the whole session 3600 seconds.

A single-pass custom alternative remains available:

```powershell
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -CaseId wanderungen-fixed-width -Repetitions 1
```

One repetition alone cannot establish repeatability; the profile defaults to two.

## Prepared profiles — run only the question currently needed

The catalogue is `capture-question-profiles.json`. There is no automatic chain
from one profile to another and no full Cartesian product over the corpus.
All scopes are resolved against the actual workbook catalogue before Excel starts.
Leading/trailing spaces in names remain significant; hidden worksheets are not
made visible. Unknown or empty selections fail instead of falling back to all.

| Profile | Exact scope | Variable / default | Repetitions | Primary cases | Session cap |
|---|---|---|---:|---:|---:|
| `fixed-width` | Wanderungen: `Vorbemerkung`, `Anhang`, `Qualitätsbericht` together | .NET / Selective / 16 MP | 2 | 2 | 1 h |
| `tile-budget` | Hospital: ` KHV_2022`; Kernhaushalt: `csv-71717-22` | 16 / 32 / 64 MP; Rust / Selective | 2 | 12 | 3 h |
| `hosts` | Wanderungen three sheets; hospital ` KHV_2022`; Kernhaushalt `csv-71717-22` | .NET / Rust; Selective / 16 MP | 2 | 12 | 3 h |
| `overflow` | Hospital: positive-overflow group `Inhalt`, `DSB_KHV_2022`, `DSB_RHV_2022`; separate ` KHV_2022` scope | Heuristic / Selective; Rust / 16 MP | 2 | 8 | 2 h |
| `startup` | Seeschifffahrt's actual first worksheet; Verkehrsunfälle `1.1` | .NET / Rust; Selective / 16 MP | 3 | 12 | 2 h |
| `coverage-gaps` | Kernhaushalt: visible sheets after `csv-71717-22`; Verkehrsunfälle: visible sheets after `1.1` | .NET / Selective / 16 MP | 1 | 2 | 3 h |

Case counts use the defaults. These are caps, not predicted runtimes. Higher-MP
fallbacks are additional attempts within the same session cap. Existing timeouts
are still finite; a heartbeat cannot renew an absolute deadline.

After `fixed-width` is confirmed, the main unanswered performance questions can
be addressed by `tile-budget`, then `hosts` at the budget under consideration,
then `overflow` at the host/budget under consideration. A non-varied field can be
overridden, but a field varied by the profile cannot be collapsed to one value:

```powershell
# Budget question; no host or overflow Cartesian product.
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Profile tile-budget

# Example only: substitute the budget supported by the budget evidence.
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Profile hosts -TileMegapixels 32

# Example only: hold the host and budget fixed while comparing overflow methods.
& .\docs\visual-validation\experiments\decision-campaign.ps1 `
    -Focused -Profile overflow -HostKind rust -TileMegapixels 32
```

The examples do not preselect 32 MP or Rust as winners. Different profiles are not
one fully controlled factorial experiment. Runs are paired only within the same
resolved scope, and arm order rotates deterministically between repetitions and
targets. It is not a complete counterbalancing or randomization scheme.

`startup` is a bounded check of previously intermittent starts, not proof of a
particular initialization defect or reliability for 10,000 launches.
`coverage-gaps` is a one-pass follow-up on the previously uncompleted tails, not a
host race. It derives the later visible sheet names from the current catalogue;
it neither assumes a fixed worksheet count nor revisits the anchor sheet.

## Recovery accounting and quality gates

Only `tile-budget` enables a single fresh 16-MP recovery after a failed or
unverified larger-budget attempt. It uses the **same exact worksheet selection,
host, overflow mode and diagnostic settings**. The previous generic recovery path
lost the sheet selection; that is corrected by preserving the candidate's scope.

The policy cost includes the failed attempt's elapsed time (including timeout and
cleanup) plus the fresh recovery workload. It never substitutes an old cached
16-MP result. Primary failures and both case reports remain visible. A policy can
be recovered while the execution ledger still correctly records a failed primary.

The question report only names a fastest measured configuration when every
required scope and repetition has completed with canonical pixel evidence,
compatible recorded Office/display environment, stable repeats and equal paired
outputs. Missing, timed-out, incomparable or different-output cases keep that
question open. It never treats failure as a fast zero-duration result.

Displayed workload includes preparation, Excel startup and capture, but excludes
offline pixel verification and report creation. Startup and capture phase intervals
are also listed. Timing categories may overlap. These are practical paired
observations, not significance tests or estimates of persistent-runner throughput.

No profile automatically writes a production choice to `decisions.md` or changes
Original/Shrink baselines. Selected-sheet agreement is not an external correctness
oracle and is not a full-corpus original-versus-shrunk regression result.

## Reports, replay and lifecycle

Each profile produces one normal focused run and one verified
`review-metadata.zip`. The root report links to:

* `question.md`, `question-summary.json`, `question-cases.csv` for this question;
* the existing per-workbook report, calibration records and raw/final images;
* the existing lifecycle, scope, timeout and reproduction records.

The whole-run reproduction request keeps the profile. A case's `reproduce.ps1`
freezes that one case's actual scope and configuration instead, including a 16-MP
recovery where relevant. It cannot accidentally restart all profile arms.

The lifecycle observer detaches an owned process handle before disposal and keeps
its last successful sample. `process-exited` is used only when exit was observed;
otherwise the status is `process-handle-released`. Later report snapshots do not
access a disposed process object. `capture_status` is distinct from
`result_transport_status` and lifecycle `result-received` / `finished`, so a
completed result transfer does not turn a partial capture into a successful one.

## Experiment ledger and deferred work

| Question | Current treatment |
|---|---|
| Adjacent image outer-edge discrepancy | Visible-cell context and exact interior crop retained; do not rerun the synthetic matrix for this patch. Existing tested reference results stay historical evidence. |
| Inconsistent `top`-derived height | Additive measured geometry retained; successful prior targeted cases are not copied into new observations. |
| Large `csv-71717-22` | Prior full targeted capture exists. Revisit only for the explicit budget/host question, not to reproduce the old whole-workbook deadline. |
| Fixed horizontal interval | New implementation requires the above three-sheet native check. |
| Rust/.NET, 16/32/64 MP, overflow trade-offs | Explicit profiles above; no automatic global recommendation. |
| Start reliability | Finite repeat profile and separate lifecycle evidence, no hidden retries. |
| Remaining workbook tails | Optional exact `coverage-gaps` profile; does not certify already untested data. |
| VBA/GDI+ references and C# drawing bounds | Preserve reports and limitations. Not rerun or repaired here; no missing equal-work comparison is called successful. |
| Productive regression over Original/Shrink | Still a separate subsequent task. No baselines are replaced by this patch. |

No experiment source or old report is deleted. The existing finalization and
historical suite commands remain available, but are not the troubleshooting entry
point for this repair.

## Validation

Pure/model tests cover full-span controls at different raster scales, independent
Y scale, repeated-width and holdout drift, zero-width neighbours, exact interval
restrictions, context crop, image budgets and strict pixel comparison. Additional
tests cover profile selection, bounded scope, arm order, policy recovery costs,
exact-scope fallback, replay, quality gates and report/archive output.

The focused Windows preflight also runs `tests/focus-observer.tests.ps1`, a small
mock-process observer test (no Excel or COM), then the existing local pixel checks.
It does not assert native renderer success. Run the actual three-sheet profile to
confirm Excel, WPF/PowerShell integration and the new calibration on these files.

The supplied Source Context Explorer archive did not include `.vscode/tasks.json`;
this patch neither creates nor edits that unexported file. The existing task may
still be used, and the explicit PowerShell commands above are the independent entry
points. No toolchains, package versions, host protocols or certificate settings are
changed.
