# Configured comparison suite

## One entry point

Run **Tasks: Run Task → Visual validation: Run comparison suite** in VS Code.
This starts Windows PowerShell automatically when the workspace is open in WSL.
The existing individual GUI/CLI experiments remain available and unchanged.

The single configuration file is `suite.json`. Its default `recommended` profile
contains four current capture candidates: Strategy F with .NET/Rust and
heuristic/selective overflow. All use **Excel /x, logical batch 1, 16 MP,
budget-aware row packing and a 1920 px right probe**. The existing capture
policy disables gridlines and retains/crops the probe as before.

The default corpus is the hospital workbook. Extend the explicit `workbooks` list
for representative multi-workbook evidence, or use `["*"]` to freeze every XLSX
under `corpus_root`. The runner rejects ambiguous/duplicate basenames. Inputs are
copied once into the suite directory, hashed, and checked before and after each
case. Original workbooks are not modified. Corpus choice is independent of the
last GUI selection; `suite.json` is the source of truth.

## Profiles, not a Cartesian product

| Profile | Invocations | Purpose |
|---|---:|---|
| `recommended` (default) | 16 | Four configurations × (one warm-up + three measured rounds). |
| `inventory` | 18 | Each relevant experiment family/configuration once; screening only. |
| `diagnostics` | 4 | Units, markers, row heights and hospital plateau only. |

Change `default_profile` in `suite.json`, or run `suite.ps1 -Profile inventory`.
`-PlanOnly` prints the expanded plan without opening Excel or creating results.
`-NoOpen` suppresses the final browser open. No unattended confirmation loop is
introduced: missing prerequisites appear as errors/skips, not successful cases.
Optional native components can still raise OS/Office dialogs; the case timeout
bounds them. Establish local .NET certificate trust and the required native
add-in/macro permissions through the existing setup before an unattended inventory.
The suite does not change security policies or enable macro trust.

Inventory includes F × both hosts/modes, A–E on .NET, VBA CopyPicture, the native
shape base group, VSTO, Office.js shape bounds on both hosts, and four diagnostics.
The native shape base group already couples VBA, Excel-DNA, PowerShell COM and
Rust COM; it is invoked once. VBA/Office.js capture policy differences are visible
and are not treated as equivalent baselines. Legacy D/E probes keep their safety
guards; a guarded failure is useful coverage evidence, not a reason to bypass limits.

## Isolation and progress

All cases are sequential. The existing global execution lock belongs to the suite.
Each case has its own worker PowerShell process, output tree, transcript and report.
Before a worker is released, it is assigned to a Windows Job Object with
KILL_ON_JOB_CLOSE. The controller may terminate only that owned job on timeout.
A waiting worker does not start experiments until attachment succeeds.

The suite refuses to start if Excel is already open or ports 3000/3001 are occupied.
It never closes unrelated Excel processes. If a COM/DCOM server outside the owned
job remains after a case, the suite stops and records the remaining cases as not
run; it does not kill arbitrary processes to force the next benchmark to run.
Windows jobs cannot promise ownership of independent COM servers or Linux
processes created via WSL. Existing experiment cleanup remains responsible for those.

The timeout in `suite.json` covers the whole worker (including offline verification
and report work); it is a safety ceiling, not a renderer-limit measurement. Normal
case failures retain their artifacts and permit the next case after idle checks.
Input drift, leftover processes or an occupied port stop the remaining queue.

`run.json` and `suite-state.json` exist from the beginning. The overview is refreshed
after each case, and the terminal prints a heartbeat plus the current log path.
A hard parent termination closes the job; **Recover interrupted runs** in the
existing report UI recovers available child reports and marks unfinished work.
Pending work is not silently resumed. Rerun the suite or narrow its configuration
explicitly; warm-up/measurement grouping must remain interpretable.

## One history entry and one upload

```
docs/visual-validation/results/runs/<suite-id>/
    report.html                  # suite overview / comparison gates
    suite-state.json             # full normalized comparison data
    case-results.csv
    result.json
    review-metadata.zip          # single compact bundle to share
    inputs/
        suite-config.json
        workbooks.json           # original paths + SHA-256
        source-fingerprints.json
        corpus/*.xlsx            # local frozen input copies
    cases/<sequence-candidate-phase-round>/
        report.html              # existing worksheet inspector
        case-metrics.json
        console.log
        worker.stdout.log
        worker.stderr.log
        data/...
        artifacts/...
```

A suite creates no second, multi-gigabyte full ZIP of all child PNGs. Images remain
in the local case directories. The routine upload is the top-level
`review-metadata.zip`; it includes all child metadata, not child archive copies.
Individual case reports link back to the suite. The existing retention UI treats
the suite as one managed run; no prior results are deleted by this patch.

## Interpretation and validity gates

- Successful measured invocations enter the median/min/max; failures stay visible
  in the denominator. Warm-ups are excluded. Warm-up is cache preparation, not a
  guaranteed cold-cache control. Order rotates cyclically; three rounds over four
  variants do not completely balance positions and are not randomized.
- `workload_ms` is measured before offline pixel hashing/report generation. It
  includes host setup/build checks, launch, preplanning, capture and existing
  diagnostics/telemetry. It is not identical to the older `capture_total_ms`, which
  omitted several phases. Named timers overlap and must not be summed.
- .NET HTTPS versus Rust HTTP and host-specific logging/encoding are part of each
  tested configuration. This is not a clean language-only Rust/C# speed experiment.
- Same-round F comparisons match workbook, exact sheet name, tile rectangle,
  raster dimensions and offsets. Different bounds/tiling are separate from pixel
  mismatches. No stitching, resizing or tolerance is used to invent equivalence.
- When final PNGs have different file hashes, measured cases can compute a
  normalized `vv-bgra32-v1` pixel digest using Windows WPF, outside workload timing.
  PNG headers/budgets and file hashes are checked first; files are never changed.
  Unsupported or missing images stay unverified. Identical PNG bytes also imply
  identical images. Digest equality is pairwise evidence, not a completeness oracle.
- Diagnostics are displayed as completed attempts with their native reports, not
  ranked as screenshot competitors. No automated winner selection or deletion occurs.
  Review the native diagnostic conclusions and original/shrink regression coverage.

## Current evidence motivating the suite

The uploaded .NET run (2026-10-05) and Rust run (2026-10-10) both report 11 complete
sheets, 70 accepted tiles, 44 calibration images, 9 completed selective checks and
21 confirmed blockers. All 70 raw capture SHA-256 values agree. Cropped final file
hashes differ on 67 tiles; those metadata bundles contain no pixels with which to
recheck the normalized output.

They are not controlled timing replicates: .NET used Excel /x and logical batch 1;
Rust used COM and logical batch 64, although both report native image batch 1.
The screen sizes and dates differ, and the .NET run built a new host. Workbook
elapsed times were 149.40 s and 220.01 s respectively. This is a configuration-level
observation, not proof that one host implementation is intrinsically slower.

Routine reduction candidates: omit the ineffective native image-batch sweep,
retain the old balanced packing mode as an opt-in baseline, keep geometry and
limit diagnostics outside routine capture comparisons, and call the coupled shape
base set only once. Broader retirement needs repeated equivalent results over a
representative corpus; the hospital workbook alone does not establish redundancy.

## Tests

```
node docs/visual-validation/experiments/tests/suite-report.test.cjs
powershell.exe -NoProfile -File docs/visual-validation/experiments/tests/suite-plan.tests.ps1
```

Node tests cover report math and equivalence classification. PowerShell tests cover
plan expansion only, not Office. Windows Job Object attachment, native C# WPF pixel
hashing, PowerShell 5.1 execution, native add-in setup and the actual Excel suite
must be verified on Windows. The patch installer does not execute Excel.

## Primary API references

- VS Code tasks: https://code.visualstudio.com/docs/debugtest/tasks
- Windows Job Objects: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
- Start-Process: https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process
