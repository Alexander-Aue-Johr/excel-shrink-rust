# Concurrent worksheet capture

`benchmark.py` runs the same workbook twice, first with one worker and then with
two workers, alternating the order on the second repetition. It requires the
capture launcher, an explicit workbook and capture configuration, and an output
directory. Capture outputs and timings are written there.

```text
python benchmark.py --launcher <run.ps1> --workbook <workbook.xlsx> --config <config.json> --output <output> --repeats 2
```

## Laptop trial, 2026-10-21

The synthetic workbook had 1,000 rows and 12 numeric columns. Capture used a
32-megapixel limit and an inactive Windows desktop for each independent Excel
instance.

| Workers | Two captures | Outcome |
| --- | --- | --- |
| 1 | 28.55 seconds | Both completed: 14.20 and 14.35 seconds |
| 2 | — | One completed; the other add-in exceeded the 120-second startup limit |

The trial stopped at the failed concurrent run. It does not establish a
throughput improvement. Keep corpus runs serial until concurrent add-in startup
is reliable on the target machine. The normal paired comparison keeps its two
workbooks together on one inactive desktop; these independent sessions each
used their own desktop.

## Repeat after capture runtime fixes, 2026-10-21

The same 1,000-row, 12-column workbook was captured twice per condition with
the launcher from main commit `2ff144e`. One serial and one parallel condition
were run; there was no further repetition.

| Workers | Two captures | Outcome |
| --- | --- | --- |
| 1 | 18.13 seconds | Both completed: 9.37 and 8.76 seconds |
| 2 | Incomplete | One completed; the other add-in exceeded the 120-second startup limit |

The launcher reported `Office add-in did not start before the startup timeout.`
Its cleanup completed; a subsequent Windows process query found no remaining
Excel, capture host, or PowerShell launcher belonging to the captures.

This measures independent single-workbook captures, not two complete
original-versus-shrunk comparisons. The parallel condition did not produce a
valid throughput measurement. Corpus comparisons therefore remain serial.

## Paired image comparison experiment

The new entry point is run.py. Each worker owns two separate Excel processes:
an original workbook and its maximum-cleaning output. All task panes connect to
one .NET host at one origin during a trial. Excel windows share an inactive
desktop. The host waits for startup to become quiet before releasing the capture
barrier. A task pane replacement after its first capture data is a failed trial.

The runner exports the capture runtime from main commit 6a0ef26 into its local
output folder. Checked adapters add bounded batching and the experiment transport;
the original geometry, visible context, selective overflow checks, exact
restoration and cropping remain in use. The main checkout is not edited.
The CLI binary must be built from main with the current maximum-cleaning flags.
Its SHA256 and the source, output, runtime and experiment-source identities are
recorded in manifest.json.

### Conditions and correctness

- 1 through 8 pairs mean 2 through 16 independent Excel processes.
- Each side has its own manifest identifier, URL client key and workbook name.
  The add-in verifies its actual workbook name before starting.
- Images are limited to 32 MP and 30,000 pixels per edge. This experiment keeps
  that budget fixed; failures are retained rather than silently mixing budgets.
- Scope arguments explicitly select sheets and A1-based rectangles. They are
  measured selections, not certificates of whole-workbook equivalence.
- Only image comparison runs. Row/column geometry and overflow verification
  remain necessary capture work; value checks and row-height early exits are off.
- images always uploads both PNGs as binary HTTP bodies. The host compares exact
  decoded pixels in a bounded worker pool.
- hash first sends dimensions, coordinates and SHA256 of PNG bytes. Matching
  digests skip upload. Different digests request those same retained PNGs and use
  the exact pixel comparator. An encoding difference alone is not a visual change.
- The host locates changed pixels and occupied 256-pixel blocks from the original
  captures. Cropped evidence is written only for differences; no recapturing or
  whole-sheet image concatenation is needed.
- A side waits for its partner after each tile. At most one bounded batch of PNGs
  can wait in that task pane. Requested batch sizes are 1, 2 or 4; an aggregate
  pixel budget additionally limits the effective batch.
- Distinct pair states, complete tile coverage and both completion messages are
  required. An incomplete capture never counts as equality or throughput.

### Run on native Windows

Prerequisites: Git, Python 3.12 or newer, Windows Excel, and a Windows .NET 8 SDK
with the Windows Desktop runtime. The existing localhost HTTP setup is reused.
Source workbooks, generated variants, source exports and all raw results stay
under local output folders.

From the repository root, with an already built main CLI:

~~~powershell
$experiment = 'docs/visual-validation/experiments/concurrent-office-capture'
$common = @(
  '--repo-root', '.',
  '--workbook', 'corpus-local/original_destatis_xlsx_files/krankenhausverzeichnis-3500100227005.xlsx',
  '--cli', 'target/release/excel-shrink-cli.exe',
  '--scope', ' KHV_2022!A1:FP400'
)

# Negative and positive controls. Repeats 0 runs only the startup pilots.
python "$experiment/run.py" @common --pairs 1 --repeats 0 --control equal --output .tmp/parallel-equal
python "$experiment/run.py" @common --pairs 1 --repeats 0 --control changed --output .tmp/parallel-changed

# Batch pilot on the hash transport, before varying concurrency.
python "$experiment/run.py" @common --pairs 1 --transports hash --batches 1,2,4 --repeats 3 --max-batch-pixels 128000000 --output .tmp/parallel-batches

# Main matrix. Substitute the qualified batch size if the pilot selects another.
python "$experiment/run.py" @common --pairs 1,2,3,4,5,6,7,8 --batches 1 --repeats 3 --max-batch-pixels 128000000 --output .tmp/parallel-matrix
~~~

Batch selection rule, declared before evaluating the pilot: among settings with
three complete repeats and matching visual findings, choose the smallest batch
within 5% of the highest median comparisons/minute. Retain the same aggregate
pixel budget when comparing requested batches. Do not call a requested batch of
4 an effective batch of 4 when the pixel limit reduced it to 2.

Repeat the matrix separately for another explicit worksheet/file selection;
do not pool different workloads into one scaling curve. The default aggregate
batch budget is 64 MP; 128 MP is the maximum permitted pilot budget.

### Timing and limits

The primary measure is completed A/B comparisons per minute, after every add-in
is ready. Startup and total wall time are separate. The capture interval includes
planning, calibration, Office rendering, overflow analysis, transport and pixel
comparison. Shrinking happens once before the trials and is excluded.

measurements.csv contains one row per observation: setup/capture/total seconds,
throughput, job CPU seconds, peak job private memory, summed Office/native capture,
hash and partner/upload waiting times, transmitted PNG bytes and effective batch.
Office timings include context.sync and IPC; they are not a direct measurement
of an internal Excel renderer alone. Waiting includes the partner and comparator.

Pilot observations (repeat 0) establish startup and the visual reference. They
are excluded from the measured medians. Measured configurations run in seeded
random order on each repetition. Different workers and transports must reproduce
the same visual findings.

Each wave has finite startup/capture deadlines and an owned Windows cleanup job.
The default private-memory budget is 65% of currently available physical RAM,
capped at 32 GiB. Physical availability below 1 GiB stops the trial. After a
completed pilot, its observed per-pair peak predicts subsequent waves with a 15%
margin plus 128 MiB. A wave exceeding its budget or leaving less than 1 GiB free
is recorded as resource-limited before Excel launches. This is a conservative
admission rule, not evidence that Excel cannot scale to that worker count.
Use --memory-mib only with a deliberate budget on the target computer.

Every trial has immutable inputs and its own result.json/observation.json and
launcher logs. The HTML table shows complete repetitions and lists failures;
incomplete or resource-limited observations do not enter throughput medians.
Peak job memory includes owned Excel/WebView and host processes, not a claimed
machine-wide RAM peak.

### Tests

~~~powershell
powershell.exe -STA -NoProfile -File "$experiment/tests/run.ps1" -RuntimeRoot '<prepared runtime folder>'
node --test "$experiment/tests/transport.test.cjs"
~~~

The Windows protocol tests cover exact equality, equal pixels with different PNG
bytes, changed pixels, missing/extra tiles, pair barriers and eight independent
sessions. Real Excel controls additionally verify that an A/A comparison stays
equal and an injected A1 text change is found by both transports.
