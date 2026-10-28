# Running corpus studies

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

## Preparation and visual scope

The study prepares and shrinks **all selected workbooks first**. Only after
every preparation has a final result does it fix the visual comparison queue
and start comparisons. The Observatory shows separate preparation and visual
totals. Setting the comparison group target to zero pauses new visual
comparisons; preparation continues. An explicit Stop ends the run after the
current workbooks and retains their completed preparation records for a restart.
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
deferred full scan** for a selected workbook, or **Stop** to end the runner.
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
in the Windows workspace printed by the WSL launcher. Install the command
once, then open a new Windows PowerShell terminal:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\install-corpus-command.ps1
```

## Run from a WSL checkout with WSL shut down

The source is in WSL, while Excel runs on Windows. First, **from the repository
root in a WSL terminal**, prepare the selected corpus:

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
excel-shrink-visual-validation -Standalone -Corpus Destatis -Partition 3/4
```

`-Standalone` selects the prepared Windows configuration and executable. You
may omit `-Partition` here to use the partition saved in that configuration.
Pass `-Partition all` to select the entire corpus explicitly. Prepare each
additional corpus by repeating the WSL preparation with its name. The study
requires Windows Excel, but does not require WSL to remain running or a
Windows Rust or .NET 8 SDK installation.

## Rebuild after a pull

1. In the Observatory, select **Stop run** for any active study. Wait until
   its active workbooks finish and the study reports that it has stopped.
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
   excel-shrink-visual-validation -Standalone -Corpus Destatis -Partition 3/4
   ```

   **Keep WSL available during the study:** Synchronize the updated source to
   the Windows workspace:

   ```bash
   bash tools/start-corpus-wsl.sh -PrepareOnly
   ```

   Then start the regular command from Windows PowerShell with `-Build`. It
   cross-compiles the Windows executable in WSL before starting the study:

   ```powershell
   excel-shrink-visual-validation -Corpus Destatis -Partition 3/4 -Build
   ```

Use `-PrepareStandalone -SkipBuild` only when a suitable Windows executable
has already been built. After pulling code changes, prepare without
`-SkipBuild`. `-Build` on a standalone command does not rebuild; prepare again
from WSL instead.
