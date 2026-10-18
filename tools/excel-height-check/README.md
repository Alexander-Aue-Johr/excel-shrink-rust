# Height and metadata comparison

These tools compare original and shrunk workbooks through Excel/VBA. They do not
select a screenshot strategy. For image capture, use
[Visual validation](../../docs/visual-validation/README.md).

## Height checks

The VS Code **Excel tools: …** tasks run the current Release CLI and optionally
compare heights: Shrink only, Default heights, Maximum Cleaning heights, All profile
heights, or Default heights after Excel save. Select an exact XLSX filename or `*`.

```sh
python3 tools/excel-height-check/run.py --profile default --height
python3 tools/excel-height-check/run.py --profile all --height --only-file foerderprogramme-5411206209005.xlsx
python3 tools/excel-height-check/run.py --profile default --height --resave-originals
```

For each shrunk worksheet, the VBA helper takes its UsedRange and measures that
same address in the original. `Range.Height` is in points. The additional ×4/3
quantity is a 96-DPI pixel equivalent, **not a measured screenshot dimension**.
Equal total height does not rule out compensating changes in individual rows.
Two VBA batches per workbook/profile avoid external COM calls for every worksheet.

Results remain under `.tmp/height-checks/<id>/`: `manifest.json` records source
hashes, profiles and shrink timings; `report.md` summarizes the run;
`differences.csv` contains differences and `comparison.json` the measurements.
Profile directories use short IDs; the manifest maps them to profiles. Exit 0
means completed without height differences, 2 means a height difference; other
codes indicate execution failure, not successful equivalence.

The optional resave control saves only original **copies**, closes and reopens
those copies before measuring. Shrink still uses the unchanged input originals.
A failed measurement does not produce a full-success certificate.

## Interactive metadata diff

Run `json_diff.py` directly. Without `--input` it uses the existing Windows Forms
file/folder picker; with `--input` it is noninteractive. This picker belongs to the
metadata workflow and is independent of the screenshot experiment WPF interface.
The current **Experiment: visual validation** task does not launch this tool.

```sh
python3 tools/excel-height-check/json_diff.py --input /path/to/file.xlsx --profile default
python3 tools/excel-height-check/json_diff.py --input /path/to/folder --profile maximum-cleaning --no-diff
```

A selected folder supplies its immediate XLSX files, excluding `~$` locks; it is
not recursively traversed. The tool builds the CLI, shrinks input copies, measures
metadata in an owned Excel instance, and opens `original.json` versus `shrunk.json`
with `code --diff` unless disabled. Results remain under `.tmp/excel-json-diff/<id>/`.
The manifest maps short input IDs back to filenames. Paths, times and hashes are
recorded separately, not injected as artificial metadata differences.

The metadata includes worksheet index/name and visibility, each workbook's own
UsedRange and its dimensions in points, standard row height, and standard column
width in Excel character units. `shrunk_range_comparison` additionally measures the
shrunk UsedRange in both workbooks. Different own UsedRanges alone are not a layout
regression. See [JSON_DIFF_NATIVE.md](JSON_DIFF_NATIVE.md) for the measurement schema.

## Requirements and helper assets

Use Windows desktop Excel, Windows PowerShell 5.1, Python 3 and the existing WSL
Rust/Cargo toolchain. Windows Python is supported when the repository is on its WSL
UNC path; this runner does not support an ordinary native Windows Rust checkout.
From WSL, `powershell.exe` must be reachable. Input Windows paths are accepted by
the metadata tool. `code` is needed only to open the diff automatically; the two
JSON files remain available when editor launch fails.

Keep `HeightProbeHost.xlsm` with `HeightProbe.bas`, and `JsonDiffProbeHost.xlsm` with
`JsonDiffProbe.bas`. These prebuilt helper workbooks are required versioned inputs.
Changing a BAS file does not update the embedded macro: rebuild/review the matching
helper before using it. The runner records helper hashes. It does not enable
AccessVBOM or change global macro security. Only the helper is opened with macros;
source workbooks stay unchanged and owned Excel instances are closed by the runner.

The profile flags correspond to `excel-shrink-cli/src/visual_profile.rs`; keep the
runner's `PROFILES` in sync. Runtime measurements and working copies are not source
artifacts and should not be committed.
