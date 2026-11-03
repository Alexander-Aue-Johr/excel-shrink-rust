# Running corpus studies

The experiment branch also provides [aggressive-first screening followed by full validation](aggressive-first-study.md).
It also supports preparation-time hash screening. In the Windows terminal launcher,
select one profile with `-PreparationHashProfile default`,
`-PreparationHashProfile maximum-cleaning`, or
`-PreparationHashProfile too-aggressive-cleaning`. The direct CLI option is
`--preparation-hash-profile` and requires `--resident-capture`.

New preparations build only the variants needed by the selected mode:
`default` builds Default only, `maximum-cleaning` builds Default and Maximum,
and `too-aggressive-cleaning` builds all three for possible targeted follow-up.
Existing compatible bundles with additional variants remain reusable. A later
request for a profile absent from a partial bundle requires new preparation.
Comparisons unpack and audit only the variants required by the current mode.
Resident captures use `fixed-right-margin-1920-v1`: every sheet extends at least
1920 measured pixels beyond the common Original/variant content boundary, rounded
up to whole columns. Hidden and narrow columns extend the measurement window as
needed. The Excel grid edge can limit that margin; measured per-sheet margins are
stored in the hash evidence. This mode performs no temporary overflow blocker
writes and takes no blocked/restored comparison images. Content beyond the margin
is not verified. Vertical tall-sheet selection remains in effect.

Earlier hash strategies are incompatible and are swept again, including previously
hash-equal results. Shrink ZIPs remain reusable; their files are not modified by
this capture-policy change. Keep a backup of previous hash sidecars before resuming
if their historical results need to be retained. The legacy classic capture path
still has its original overflow checks; this experiment uses resident capture.

Preparation has two corpus-wide phases. First, every selected workbook is
converted if necessary, shrunk, and saved in its reusable ZIP bundle. Only after
that queue drains do hash sweeps open Original and the selected output, capture
every tile, run Excel Full recalculation in both workbooks, and capture again.
The Observatory shows separate shrink and hash-sweep progress bars. Beneath them,
each live Excel process has one retained progress row. It counts completed sheets
across both hash passes (for five sheets, 0 through 10 sheet passes), labels the
current sheet and pass inside the bar, and stays visible between commands. A sheet
transition updates that existing row instead of adding a per-sheet bar. Processes
that are still opening have an indeterminate row until their sweep extent is known.
The same row also shows completed/planned captures for the current sheet. Its
capture count resets on sheet changes; sheet-pass progress remains cumulative.

The Observatory's **Captures per sync** slider accepts 1–1024. It applies to newly
opened workbook sessions, including autonomous hash sweeps. Sweeps queue at most
64 megapixels of same-sheet images per `context.sync()`, hash each returned PNG,
and release its bytes before the next batch. Small tiles can share a much larger
batch than large tiles. The evidence-retention paths still cap batches at 16 images.
`EXCEL_SHRINK_CAPTURE_BATCH_TILES` selects the same limit for standalone captures.

Office image results are unavailable until `context.sync()` completes; a full
workbook sweep with no synchronization cannot read or hash its images. HTTP
heartbeats run independently every five seconds while Office is awaiting a sync.
The capture count advances as returned images are finalized, not while Excel is
still rendering a pending batch. Full recalculation remains a separate confirmed
step between sweeps and waits for Excel's calculation state to become `Done`.
Capture timing evidence includes `capture_syncs` and `max_capture_batch_tiles`.
The ignored `default_only_preparation_excel_fixture` test can compare batch sizes
1, 16, 1024 and 1 on the same prepared ZIP when
`EXCEL_SHRINK_SWEEP_BATCH_BENCHMARK=1` is set; it verifies identical global hashes
and an unchanged ZIP SHA-256 and writes `batch-benchmark.json` in its test output.

`prepare_processes` controls simultaneous shrink jobs; `shrink_threads` sets
Rayon threads per shrink job. `sweep_processes` independently controls concurrent
hash-sweep groups, each normally using two Excel instances. The comparison-group
target does not gate sweeps. There is no separate limit on owned Excel instances.
The sweep slider alone controls simultaneous sweep jobs; each job opens the Excel
instances it needs. Retired `excel_process_limit` settings in saved runs are ignored.

Sweeps save both passes, per-tile and per-sheet SHA-256 hashes, and a global hash
in a `*.capture-hashes.json` sidecar beside the preparation ZIP. The sidecar is
bound to that exact bundle's SHA-256 and selected profile. Existing bundles are
not rewritten; embedded hashes from older bundles remain readable. Preserve the
sidecars when copying a run to reuse its new sweeps. Missing or invalid caches
remain eligible for capture; they never establish equality. Failed sweeps retain
the preparation and diagnostic error and fall back to selected-profile visual
comparison rather than losing the shrinking work.

Each new sidecar also contains `screen.diagnostic_timings`: `first` and `second`
separate Original and the selected profile, with per-sheet planning, capture,
image-finalization, hash and elapsed times. `recalculate` records the command
times between passes. `startup` measures Excel launch through add-in readiness,
not `Workbooks.Open` alone; `coordinator.wait_ms` is cumulative time waiting for
events and can overlap worker activity. These diagnostics do not enter the
stability decision or global capture hash. Older sidecars have no timing section.
`hash_ms` is included in `finish_image_ms`, and the per-sheet elapsed time
includes setup and planning; these fields must not be summed.

Only stable hash differences select sheets for later pixel comparison. A missing
or unstable manifest selects all sheets of the chosen profile. Hash equality is
reported as `screened-equal`; it is not a complete visual or cell-value check.
`default` stops after that profile. `maximum-cleaning` adds targeted Default
captures if Maximum has visual differences. `too-aggressive-cleaning` adds
targeted Maximum and Default captures if Too Aggressive has visual differences.
Targeted follow-up captures do not run independent recalculation validation.

The two sweeps detect changes caused by an explicit Full recalculation in that
Excel session. They cannot rule out later refresh-on-open behavior, external
data changes, or a different rendering in a new Excel session; the report
therefore keeps screened results separate from complete visual validation.
See the [two confirmed Default formatting regressions](confirmed-default-regressions.md) for the controlled findings.

This guide assumes you have saved a corpus configuration. If you have not,
run **Excel Shrink: Configure corpus run** in VS Code. The full
[launcher guide](corpus-launcher.md) covers setup, archive binding, and
additional options.

These commands start the regular visual-validation study. The isolated quartet
benchmark on the experiment branch has a separate runner.

## Choose a partition

In **Windows PowerShell**, use the installed command to select a corpus and
partition:

```powershell
excel-shrink-visual-validation -Corpus Destatis -Partition 3/4
```

`3/4` selects the third of four size partitions; `1/4` selects the first.
`-Partition all` selects the entire corpus. Partitions divide files into groups
of approximately equal file count. Processing order is separate: it comes from
the saved configuration unless you pass `-Order size-asc` or `-Order size-desc`.

The regular command requires **both `-Corpus` and `-Partition`** and uses the
regular configuration. It opens the Observatory. Running the same command
again while that study is active opens its Observatory instead of starting a
second study.

## Example overnight assignments on native Windows

Run one assignment per computer in **Windows PowerShell**. These examples use
the installed command, each computer's saved configuration and its local corpus
ZIP. They do not require WSL.

Enron, first half (the smaller files):

```powershell
excel-shrink-visual-validation -Corpus Enron -Partition 1/2 -Order size-desc
```

Enron, second half (the larger files):

```powershell
excel-shrink-visual-validation -Corpus Enron -Partition 2/2 -Order size-desc
```

Data.gov, entire corpus:

```powershell
excel-shrink-visual-validation -Corpus Data.gov -Partition all -Order size-desc
```

Enron's halves are selected by the original workbooks' uncompressed file sizes,
not by archive order. `-Order size-desc` processes the largest files *within*
each selected half first. Update and build the intended version on each computer
before starting a night run.

## Preparation and visual scope

The study prepares and shrinks **all selected workbooks first**. Only after
every preparation has a final result does it fix the visual comparison queue
and start comparisons. The Observatory shows separate preparation and visual
totals. Setting the comparison group target to zero pauses new visual
comparisons; preparation continues. **Finish active workbooks** ends the run
after current workbooks complete. **Stop now** cancels active workers and closes
their owned Excel sessions. Completed records remain available for a restart.
The Observatory has separate controls for simultaneous shrink processes and
Rayon threads per process. Both range from 1 to the available logical CPU count;
changes apply only to newly admitted workbooks. Preparation does not gate new
processes on free RAM. The Rust worker still reports its measured memory peak.
Before each preparation, the runner checks free space on the output volume.
It pauses with **waiting for disk** when less than 8 GiB, or the larger
combined reserve for active workbooks and the next one, is available. Free
space and start the same run again to resume; completed preparations remain
available.
Unsupported XML namespace roots are classified by the shrinker; the study
uses its batch outcome instead of scanning the ZIP members again.
Worksheets containing CDATA are copied unchanged while other worksheets can
still be cleaned. The streaming validator retains CDATA text but does not
certify formulas represented that way for cache reuse.
The shrinker records ZIP member length and CRC-32 for each sheet. A profile
whose entire output reuses the input archive is listed as **CRC + length equal
to shrink input** and receives no visual comparison. This is distinct from a
visually identical result. Shrinking uses the converted corpus file. If only
`xl/workbook.xml` defined names could be cleaned, the default output retains
the source archive and the study reports **Only workbook.xml cleanup available**.
These files receive neither a repaired baseline nor a visual comparison.
Unsupported namespaces, preparation failures, and preparations missing required
shrink evidence also stay out of the visual queue, with their reason recorded
in the workbook index.
`--write-workbook-only-changes` opts into publishing the workbook-only cleanup.
When a profile changes worksheet XML, baseline repair runs after shrinking,
before package and visual comparison.

## Deferred full scans

The Observatory can initially capture only the top and bottom of a large sheet
whose length and CRC-32 match after shrinking. Its **CRC-equal sheet threshold**
control defaults to 256 MiB of uncompressed sheet XML. Omitted ranges are shown
as deferred, even if conditional formatting exists or Excel's recalculated
copy has different ZIP metadata. After normal comparisons, the Observatory
stays open in **Awaiting manual full scan** with no Excel clients. Use **Start
deferred full scan** for a selected workbook, or **Finish active workbooks** to
end the runner.
The saved state can be resumed later with the same corpus and partition:

```powershell
excel-shrink-visual-validation -Corpus Destatis -Partition 3/4 -FullScanDeferred
```

This reuses prepared workbooks; the full scan remains subject to the capture
budget of 4096 image tiles. The Observatory distinguishes these deferred
checks, CRC equality, recalculation metadata changes, visual identity, and
visual differences. Bloat category totals are collected during the measured
shrink run. The optional `--analysis-csv --disable-tail-pruning` variant is for
separate research comparisons and is not part of the visual study's timing.

## Install the Windows command

If `excel-shrink-visual-validation` is not installed, open Windows PowerShell
in the repository root (or the Windows workspace printed by the WSL launcher).
Install the command once, then open a new Windows PowerShell terminal:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\install-corpus-command.ps1
```

## Optional: release WSL memory for a Windows run

If the source checkout is in WSL and you want its memory available to Excel
during a night run, prepare a standalone Windows copy before shutting WSL down.
Destatis is the example here. First, **from the repository root in a WSL
terminal**, run:

```bash
bash tools/start-corpus-wsl.sh -PrepareStandalone -Corpus Destatis
```

This synchronizes the Windows workspace, cross-compiles the Rust CLI for
Windows, copies the selected archive to a Windows drive, and verifies its
SHA-256. It saves a separate Windows configuration and installs the terminal
command. Without `-Standalone`, that command still uses the regular
configuration.

Then, in **Windows PowerShell**:

```powershell
wsl --shutdown
excel-shrink-visual-validation -Standalone -Corpus Destatis -Partition all -Order size-desc
```

`-Standalone` selects the prepared Windows configuration and executable. You
may omit `-Partition` here to use the partition saved in that configuration.
Pass `-Partition all` to select the entire corpus explicitly. Prepare each
additional corpus by repeating the WSL preparation with its name. The study
requires Windows Excel, but does not require WSL to remain running or a
Windows Rust or .NET 8 SDK installation.

## Rebuild after a pull

On a native Windows computer, stop its active run, update the local checkout,
and add `-Build` to the next example command to rebuild before starting. The
steps below apply to a checkout kept in WSL.

1. In the Observatory, select **Finish active workbooks** for any active study.
   Wait until its active workbooks finish and the study reports that it has stopped.
   Setting the group count to **0** only pauses admission and keeps the output
   directory in use.
2. Run `git pull` in the **WSL checkout**, not in the deployed Windows copy.
3. Follow the path that matches how you intend to run the study:

   **Shut down WSL for the study:** Prepare again in WSL. This cross-compiles
   the updated Windows executable:

   ```bash
   bash tools/start-corpus-wsl.sh -PrepareStandalone -Corpus Destatis
   ```

   Then start from Windows PowerShell:

   ```powershell
   wsl --shutdown
   excel-shrink-visual-validation -Standalone -Corpus Destatis -Partition all -Order size-desc
   ```

   **Keep WSL available during the study:** Synchronize the updated source to
   the Windows workspace:

   ```bash
   bash tools/start-corpus-wsl.sh -PrepareOnly
   ```

   Then start the regular command from Windows PowerShell with `-Build`. It
   cross-compiles the Windows executable in WSL before starting the study:

   ```powershell
   excel-shrink-visual-validation -Corpus Destatis -Partition all -Order size-desc -Build
   ```

Use `-PrepareStandalone -SkipBuild` only when a suitable Windows executable
has already been built. After pulling code changes, prepare without
`-SkipBuild`. `-Build` on a standalone command does not rebuild; prepare again
from WSL instead.

## Recorded investigations

- [Enron overnight run, 2–3 November 2026](investigations/enron-night-run-20261103.md): preparation completed, visual comparisons failed, and Windows became unresponsive.

### Rebuild shrink outputs from preserved conversions

Set `EXCEL_SHRINK_CONVERTED_BUNDLES_DIR` to an earlier run's `prepared` directory
when rebuilding shrink variants from existing legacy-XLS conversions. Only the
original converted XLSX is restored: bundle identity, raw source SHA-256 and
converted-file SHA-256 must match. A mismatched or corrupt bundle fails explicitly;
a missing bundle follows ordinary Excel conversion. The workbook record retains
`conversion_reuse` provenance. Old shrink outputs are not approved by this setting.
Keep the source bundles available for the preparation phase. This setting does not
replace `--reuse-prepared-from`, which approves complete shrink preparations.
