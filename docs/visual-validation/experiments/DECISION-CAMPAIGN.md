# Bounded visual-validation decision campaign

## Run once, resume the same campaign

VS Code task and Run/Debug entry: **Visual validation: Decide strategy (all workbooks)**.
The task bridges WSL to Windows PowerShell. The controller uses Python 3.10+ and the
standard library only; the existing Windows Excel workers remain responsible for
capture. No additional Python package, WPF dependency for the suite, Windows policy
change or certificate setup is added.

`decision-campaign.json` is the one configuration. Defaults recursively inventory
**every XLSX under `corpus-local`**, excluding Excel lock files and directory links.
Exact relative paths identify inputs, so identical basenames in different folders
are not conflated. Private per-input copies preserve the original basename and are
hashed before use. Source workbooks are not saved back.

Close existing Excel instances before running. Keep the same display scaling and
monitor during the campaign. During clipboard-based reference cases do not use the
clipboard concurrently. `Range.CopyPicture` really does use the system clipboard;
this is a dependency, not an assumption that a corruption occurred.

A preflight compiles the Windows process supervisor and the offline GDI+ strip
validator, and runs six tiny-image checks **before opening Excel**. A failing
preflight must be resolved rather than consuming the corpus budget.

The normal task resumes an interrupted/partially covered campaign with unchanged
configuration, source fingerprint and corpus inventory. Completed cases are not
repeated. Interrupted attempts retain their own evidence and costs when restarted.
Finite comparison-stage budgets are not reset on resume; remaining **corpus
coverage** continues. The controller never silently turns a new source/config into
an old campaign. `decision-campaign.ps1 -NewRun` explicitly starts a new campaign.

A no-Excel plan is available with `decision-campaign.ps1 -PlanOnly`; a report-only
rebuild is `decision-campaign.ps1 -ReportOnly`. Neither is required before the normal
task. The existing single-experiment tasks are retained as historical tools.

## Question-oriented design, not every variable crossed with every other variable

This is a **blocked, staged engineering qualification**, not a complete factorial
experiment or a significance test. Workbooks are the comparison blocks. Do not pool
different file workloads into an unqualified mean or rank unlike native tasks as
identical screenshot pipelines.

| Stage | Scope | Question |
|---|---|---|
| Setup | One F/16-MP warm-up per Office.js host; one reviewed native helper build/reuse when needed | Are the existing prerequisites usable? These costs are excluded from candidate medians, not erased. |
| Controlled screening | All files when at most 12; otherwise 12 size-stratified/seeded files, including existing named stress cases | Rust/selective at 16, 32, 64 MP; both hosts and both overflow modes at 16 MP. Only six unique configurations. |
| Interaction probe | One middle-size screening workbook; the selected budget on both hosts/modes, plus the best 16-MP control and the conservative 16-MP fallback where distinct | Does the selected Rust budget also make sense with the other host/mode? Do not assume a larger budget beats a previously faster 16-MP configuration. |
| Confirmation | Only two finalists, twice on up to three screening workbooks | Are outputs reproducible and is the preference supported on paired files? The interaction probe is not silently advertised as a full corpus replication. |
| Native and historical references | Up to eight screening files, within a separate time budget | Existing A-E bounds methods, VBA chart export, VBA-local GDI+ bitmap/EMF, an identical pair with/without PNG persistence, native row scan, VBA/Excel-DNA/PowerShell/Rust geometry group, VSTO, Office.js geometry. |
| Renderer diagnostics | Units, markers, row heights and hospital plateau once per campaign | Recheck established geometry/limit assumptions without multiplying them by corpus size. |
| Every-file coverage | Every remaining input, using only the deployment candidate | Does the chosen configuration finish across the actual corpus? No factorial sweep over 10,000 files. |

The reference first pass touches different families before spending its entire
budget on all files of one family. Later reference coverage fills the remaining
file blocks. A reference excluded by a deadline/prerequisite remains in the ledger
with a specific reason. Unexecuted tests never become zero-time successes.

### Native implementation distinctions

The C# Excel-DNA and VSTO projects in this source snapshot collect **drawing bounds**.
They do not contain a separate C# screenshot acquisition implementation. Their
existing geometry validation and producer timing files are linked directly from the
campaign report. The four-way native geometry runner is invoked once per input, not
four times for the same bundled test.

The existing historical VBA chart export has its own guarded/capped plan. Its times
are recorded, but it is not treated as equivalent full Strategy F coverage.

The VBA-local GDI+ routes use the existing Excel Lab helper. Completed F sheet bounds
are passed in an explicit XML plan; no new inflated UsedRange scan is introduced to
invent capture bounds. Native tiling/raster policies are deliberately retained and
are not represented as identical to Office.js. The helper runs one `Excel.Run`
batch per file: the controller does not add an external COM roundtrip per image.

The memory test opens two unchanged working copies and compares owned GDI+ buffers.
The matching disk-pair test performs the same two acquisitions/comparison **with PNG
persistence enabled**. This isolates that persistence cost within the native route.
The single-acquisition bitmap/EMF tests are separate tasks and should not be ranked
against the pair as though they perform the same amount of work.

The helper is rebuilt from the reviewed BAS files if stale and local policy permits
it. A missing permission/build failure disables those references for the campaign;
the suite does not change Trust Center settings. Existing macro helper build logic
and its validations are reused.

### 32/64-MP failure is part of the policy cost

A larger-budget policy may perform **one fresh-session, same-host/same-mode 16-MP
recovery**. Both executions and all artifacts remain recorded. The recovery-aware
cost includes the failed primary elapsed time (including waiting/teardown), followed
by the real fallback workload. No cached screenshot or hypothetical retry time is
substituted. A timeout is a censored primary observation, never an exact successful
completion time. If the fallback also fails, the policy is not admitted as complete.

For broad coverage, a distinct .NET/selective/16-MP fallback is available if the
selected policy still fails. Each fallback is limited and visible; a series of
unbounded retries is not a deployment strategy.

The internal safe-capture tile subdivision remains unchanged. Thus the benchmark
measures the **existing implemented policy** (including its recorded retries), not
a fictional raw 64-MP call with no recovery. It does not change tile edge or canvas
limits, raster calibration, gridline policy or selective-overflow semantics.

## Deadlines and stopping rules

These are **budgets**, not promises that the entire corpus finishes in these times.

| Budget | Default |
|---|---:|
| Campaign session, including workers | 6 hours |
| Controlled screening | 2 hours |
| Interaction probe + finalist confirmation | 40 minutes |
| Native/historical references | 45 minutes |
| Renderer diagnostics | 20 minutes |
| One setup case | 15 minutes |
| One case with no matching timing | 6 minutes |
| Normal per-file workload bounds | 2 to 20 minutes |
| One historical/reference case | At most 5 minutes |
| One diagnostic case | At most 10 minutes |
| Offline pixels/report phase | At most 4 minutes per case |
| Minimum remaining disk space | 10 GiB |

When a successful workload timing for the **same input hash/file** is available,
the deadline is `median * 2.5 + 45 seconds`, clipped to the limits above and to the
remaining stage/session budget. Current-campaign baselines are preferred; historical
suite records are used for timeout sizing only. Different DPI, source versions,
hardware and measured tasks make old timings unsuitable as replacement comparison
rows. Shrink-processing baselines are not automatically treated as Excel-capture
baselines.

The Windows supervisor uses a Job Object and ownership receipts (PID plus process
start time) for COM instances where available. It has separate workload and
post-processing deadlines. A parent-process/heartbeat lease prevents a released
worker from continuing indefinitely after the campaign controller disappears.
Cleanup may require a short bounded grace period. **No unknown Excel instance or
unrelated port listener is killed.** If an external COM instance cannot be proved
owned, the campaign pauses rather than risk user work. In particular, not every
external native COM helper currently publishes a receipt; this remains visible as
a supervision limitation, not a claim of universal cleanup.

A candidate with failures/timeouts on three distinct inputs and no successful case
is pruned from the remaining screening/reference queue. Pruned and skipped cases
remain documented. Coverage uses its bounded fallback instead of an unlimited
rescue loop. Free-space exhaustion stops the queue without deleting evidence.

No finite deadline can guarantee that every comparison and every corpus file is
executed. Pending coverage is explicit; exhausted optional comparison budgets remain
closed instead of creating another endless experiment phase.

## Output equality across different tile cuts

After workload measurement, existing per-PNG verification is followed by a canonical
BGRA row-stream digest. It decodes **one bounded tile at a time**, never a giant
worksheet bitmap. The same complete pixel stream produces the same digest even when
16 and 32 MP choose different row cuts.

This validator currently supports complete full-width vertical strips. It validates
source PNG hashes and dimensions, starting offset, continuity and up to two overlap
rows. Duplicate seam pixels must be identical; they are not merely dropped. There
is no scaling, alignment, tolerance, alpha masking or majority-vote deletion of an
outlier. Horizontal/unequal-width layouts, inconsistent overlaps, missing files or
partial canonical data remain **unverified**.

This is a conservative quality gate, not a proof that Excel painted every relevant
object. Equal output from two implementations is agreement, not an external truth
oracle. A single-pass corpus capture also does not establish statistical reliability
for future 10,000-file runs.

## Decision policy and outputs

First compare quality/coverage and repeated unchanged-input consistency, then costs.
At least three paired files and at least two quality-compatible candidates are
required for a supported preference. Failed attempts are retained alongside successful
medians; the budget-policy table separately includes recovery costs.

Default engineering preferences, declared before measuring:

* Prefer Rust within 15% of the fastest quality-compatible configuration to avoid an
  additional **ASP.NET host** in production. Windows PowerShell/GDI+ helpers still
  use .NET; this is not a promise that every component is now Rust.
* Prefer selective verification within 15% for its explicit counterfactual evidence.
  That preference does not assert that the heuristic is wrong on every sheet.
* Prefer the smaller tile budget within 5% when the difference is not worth the
  larger per-image exposure. Preference tolerances do not compound beyond the
  declared overall overhead allowance.

The first screen is not enough to prove absence of interactions. The interaction
probe and repeated finalists are a bounded check, not an exhaustive interaction
model. A gate that remains open produces a **provisional operational fallback** and
an exception list, not a fabricated definitive winner.

Each campaign is one existing Run-History entry:

```
report.html                 # professional overview and direct case links
campaign-state.json         # resumable inventory, stage state and evidence
case-results.csv            # raw execution outcomes, not just successful averages
decision-summary.json       # paired comparisons and recovery-aware budget result
decision.md                 # generated English decision memorandum for author review
review-metadata.zip         # verified metadata-only bundle to upload
cases/<short-id>/            # normal case report, measurements, logs and raw PNGs
```

`decision.md` contains the proposed configuration, policy thresholds, evidence,
timeouts, skipped references and limitations. It does **not** automatically replace
`docs/visual-validation/decisions.md` or modify production regression baselines.
Copy the reviewed decision after inspecting its explicit exceptions. Generic report
refresh/pruning does not overwrite a campaign view or delete a resumable campaign.

The metadata ZIP is built under a temporary name, manifest- and hash-verified, and
only then published. The download link stays disabled until publication. Images
remain local; no manual PNG removal or 7z repacking is needed. The metadata is not
anonymized: local paths, workbook names and diagnostic cell values can be present.

A completed campaign with exceptions is a successful **orchestration completion**,
not an assertion that every experiment passed. Partial coverage/interruption returns
a nonzero task status and is resumable. Use the same task to continue it.

## What this patch deliberately does not do

It does not add another C# screenshot engine, port the pixel scanner to WASM, delete
old experiments, assert unobserved clipboard corruption, or run the final
original-versus-shrunk regression across 10,000 files. Those would enlarge the task
instead of closing the current strategy choice. Once the decision is reviewed,
freeze one production capture configuration and start the actual regression run.

## Validation

`tests/test_decision_campaign.py` checks the pure planner, gates, timeout bounds,
recovery accounting, metadata publication and a simulated staged campaign. Existing
JavaScript tests cover the reused capture/overflow/report logic. The patch installer
has dry-run, idempotence, conflict and rollback checks. These are **not** native
Windows/Excel end-to-end tests. The six native pixel preflight checks execute on the
user's Windows machine before the first Excel case. BAS compilation, Office
initialization, COM ownership and real clipboard behaviour still require that run.

Primary background references:

- Blocking nuisance factors: https://www.itl.nist.gov/div898/handbook/pri/section3/pri3333.htm
- Range.CopyPicture and the clipboard: https://learn.microsoft.com/en-us/office/vba/api/excel.range.copypicture
- Office.js batching/roundtrips: https://learn.microsoft.com/en-us/office/dev/add-ins/excel/performance
