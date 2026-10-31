# Native Excel startup across private desktops

This experiment measures whether two independent Excel processes become ready
sooner on separate private desktops than on one shared private desktop. It does
not assume that Excel serializes startup. No production corpus is stopped or
reconfigured, and no global keyboard or pointer input is sent.

## Run

Requirements: Windows desktop Excel, Python, and an existing release
`target/release/excel-shrink-cli.exe` providing the capture host. Windows
PowerShell compiles the existing Windows API helpers; no .NET SDK is needed.

```powershell
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File `
  docs/visual-validation/experiments/parallel-workbook-open/run.ps1 `
  -Output .tmp/parallel-open-demo
```

Use a new output directory. `-TargetSeconds 45` and `-Repetitions 2` are the
defaults. Calibration starts with one sheet and grows toward the duration target,
up to eight sheets. The target is a sizing hint, not an artificial sleep inside
Excel and not a guarantee that this machine needs 45 seconds.
`-Fixture path/to/generated.xlsx` reuses a generated fixture after checking its
sheet dimensions, without repeating generation or calibration.

Each sheet has exactly **1,000,000 populated rows and three numeric columns**.
Values are deterministic; there are no formulas, links, macros, or intentional
formatting. XML generation is streamed in small batches. Every workbook opens
through native `excel.exe /x`, with the normal injected Office.js add-in.

## Procedure and safety

`run.ps1` owns two private desktops and a job containing the experiment's
process tree. Its maximum duration is 30 minutes. `run.py` owns each independent
resident launcher and closes its session before moving to another trial.
Only a disposable copy of the production worker receives instrumentation:

- Explicitly select the requested private desktop.
- Publish a prepared marker after workbook staging and add-in registration.
- Wait for a shared UTC target, set two seconds ahead only after both launchers
  are prepared. Launchers independently start Excel when that time arrives.
- Record the actual request time, PID, desktop, and scheduled time.

Inherited corpus admission settings are cleared from experiment children. There
is no experiment-side global opening lock. Separate trials have unique staging,
add-in registrations, servers, and WebView2 profiles. We retain both Excel
instances until both become ready, then close both owned sessions.

At most two experiment Excel instances run together. Available physical memory
is checked throughout startup; falling below an 8 GiB reserve aborts and cleans
up the experiment. Startup and barrier waits are bounded. The production runner
is outside this job and is not terminated by experiment cleanup.

The three arrangements are measured twice:

1. Open A, close it, then open B on the same private desktop.
2. Open A and B together on one private desktop.
3. Open A and B together on two different private desktops.

The second round reverses that arrangement order. Staging copies warm the
filesystem cache in every arrangement. Existing background work, including the
production corpus, remains a source of timing variation.

## Measurement and evidence

The endpoint is **verified add-in readiness**, not a COM `Workbooks.Open` return:
the existing Office.js worker has accessed its workbook and published its
`addin_ready` event. This includes taskpane initialization and the local readiness
message. It does not separately instrument Excel's XML parser or wait for the
subsequent native startup-sample gate. `launcher_startup_sampled` records whether
that later step happened; it is not required for this experiment's endpoint.

The initial attempt used the launcher's primary-ready flag. On the second private
desktop, Office.js had already verified the workbook but native admission stayed
pending, eventually causing the peer's idle capture timeout. That attempt is
kept as diagnostic evidence, not counted as a failed or slow workbook opening.
The corrected experiment repeats all arrangements against the same fixture and
uses verified Office.js events consistently for every trial.

Parallel pair time runs from the earlier actual launch request to the later
readiness timestamp. Sequential pair time sums the two corresponding opening
intervals; setup, two-second gates, and teardown gaps are excluded in both cases.
Launch skew is reported rather than assuming that the UTC target guarantees
perfect simultaneity.

Every 250 ms, record Excel CPU time, private memory, readiness, and system free
RAM. CPU overlap counts only intervals where both exact sampled PIDs remain
pending and both increase CPU time by at least 20 ms. This supports a timing
finding; it is not direct observation of Excel's internal loading phases.

Outputs include:

- `fixture.json`: workbook dimensions, path, and calibration duration.
- `results.json` / `measurements.json`: completed trials, PIDs, start/ready times,
  peak memory, desktop names, and launch skew.
- Each trial's `samples.json`: CPU/memory observations and readiness.
- `<output>.desktops.json`: input desktop before/after and owned desktop names.
- `summary.json` / `report.html`: medians, speedups, CPU evidence, and timelines.

To regenerate the visual report or run its CPU-evidence checks:

```powershell
python docs/visual-validation/experiments/parallel-workbook-open/report.py .tmp/parallel-open-demo
python docs/visual-validation/experiments/parallel-workbook-open/test_report.py
python docs/visual-validation/experiments/parallel-workbook-open/test_readiness.py
```

The report uses the same time scale for all bars. Sequential setup/teardown gaps
are deliberately omitted. Two rounds are an engineering experiment, not a
statistically powered performance claim or a substitute for the paper's final
benchmark protocol.

## Observed results

On this Ryzen 7 7800X3D machine, the eight-sheet fixture was 199,438,289 bytes.
All eight ZIP parts passed CRC verification; the repeated worksheet XML contains
exactly one million rows and three million numeric cells per sheet.

| Arrangement | Round 1 | Round 2 | Median pair time |
| --- | ---: | ---: | ---: |
| Sequential | 50.689 s | 48.802 s | 49.745 s |
| Shared private desktop | 27.438 s | 26.860 s | 27.149 s |
| Separate private desktops | 28.595 s | 27.828 s | 28.212 s |

The actual process-creation timestamps differ by 0–1 ms in the parallel cases.
Both pending Excel processes made simultaneous CPU progress for 25.3–26.4 s.
The input desktop remained Default. Sharing one private desktop therefore did
not force serial loading in this native `/x` setup. Separate desktops provided
no additional speedup in these trials.

The native startup sample stayed pending for the second desktop in both separate-
desktop trials, although its Office.js verification succeeded. This is a launcher
admission issue to investigate separately before production changes; it must not
be interpreted as continued workbook loading.

`findings.json` retains compact results, machine information, fixture/binary
hashes, memory observations, and the native-sample caveat. Full telemetry and the
visual report remain in `.tmp/parallel-workbook-open-verified-ready` on this machine.
