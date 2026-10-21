# Regression Corpus Workflow

This directory defines the real-file regression workflow for `excel-shrink-rust`.

The rule is:

- the Rust regression runner and baselines live in this repository
- the real XLSX corpus stays outside this repository
- baselines are versioned here and are not auto-updated by the runner
- short human-facing case notes live directly in the baseline JSON

## Directory layout

```text
regression/
`- baselines/
   |- smoke.json
   `- corpus.json

.tmp/
|- regression/       # generated outputs, gitignored
|- reports/<visual-profile>/
|- workbooks/shared/<workbook-hash>/
`- baselines/<visual-profile>/
```

## Runner

```bash
cargo run --release -p excel-shrink-cli -- regression smoke
cargo run --release -p excel-shrink-cli -- regression corpus
```

Point the runner at a local corpus using either:

- `--corpus-root <PATH>`
- `EXCEL_SHRINK_CORPUS_ROOT=<PATH>`

If no corpus root is provided, the runner first checks the gitignored local cache, then a sibling `../original_destatis_xlsx_files` clone, then `download_url`.

## Runner modes

`cargo run --release -p excel-shrink-cli -- regression smoke`

- intended for frequent local checks
- contains representative category-oriented single-workbook cases
- uses `regression/baselines/<visual-profile>/smoke.json` when present, otherwise `regression/baselines/smoke.json`

`cargo run --release -p excel-shrink-cli -- regression corpus`

- intended for heavier full-corpus runs
- may contain many cases plus aggregate expectations
- uses `regression/baselines/<visual-profile>/corpus.json` when present, otherwise `regression/baselines/corpus.json`

## Baseline format

Both baseline files use JSON.

Minimal case shape:

```json
{
  "id": "empty-rows",
  "category": "Empty Rows",
  "reason": "Focused sentinel for deleting rows that no longer carry visible style or height semantics.",
  "dominant_cleanup": [
    "EmptyRowsWithoutVisibleStyleOrHeight",
    "EmptyRowAttributeCleanup"
  ],
  "relative_input": "jahresbericht-5422701207005.xlsx",
  "download_url": "https://raw.githubusercontent.com/Alexander-Aue-Johr/original_destatis_xlsx_files/main/jahresbericht-5422701207005.xlsx",
  "cached_file_name": "jahresbericht-5422701207005.xlsx",
  "min_reduction_ratio": 0.93
}
```

Human-facing case fields:

- `category`
- `reason`
- `dominant_cleanup`
- `notes`

Runner/provenance fields:

- `id`
- `source_repository_url`
- `source_file_url`
- `relative_input`
- `input_path`
- `download_url`
- `cached_file_name`
- `cli_args`
- `analysis_mode`
- `max_output_bytes`
- `min_reduction_ratio`
- `analysis_checks`

Use `relative_input` for files below the corpus root. Use `input_path` only for explicit absolute or repo-relative paths.

Use `download_url` and `cached_file_name` when the workbook should be fetched into the local gitignored cache automatically.

`analysis_mode: true` makes the runner pass `--analysis-csv`, which uses attribution-safe analysis behavior. Use this only for cases where analysis-mode expectations matter.

## Analysis checks

Example:

```json
{
  "id": "wanderungen-hidden-content",
  "analysis_mode": true,
  "analysis_checks": [
    {
      "sheet_file": "xl/worksheets/sheet3.xml",
      "cause": "UnreferencedHiddenContentCells",
      "min_removed_items": 1,
      "min_reduction_bytes_uncompressed_xml": 100
    }
  ]
}
```

Supported analysis check fields:

- `cause`
- `sheet_file`
- `min_removed_items`
- `min_removed_bytes_uncompressed_xml`
- `min_reduction_bytes_uncompressed_xml`

## Aggregate expectations

The heavy corpus baseline may also contain:

```json
{
  "aggregate": {
    "max_total_output_bytes": 123456789,
    "min_total_reduction_ratio": 0.82
  }
}
```

This is useful when the important signal is overall shrinkage across the corpus.

## Usage

Smoke run:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --corpus-root /data/excel-shrink-corpus
```

Full corpus run:

```bash
cargo run --release -p excel-shrink-cli -- regression corpus --corpus-root /data/excel-shrink-corpus
```

Use an environment variable instead of a parameter:

```bash
EXCEL_SHRINK_CORPUS_ROOT=/data/excel-shrink-corpus \
  cargo run --release -p excel-shrink-cli -- regression smoke
```

Run only one case by category ID:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --case-id empty-rows
```

Pass an extra CLI option through to each case:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --cli-arg=--store-output
```

Ignore baseline-level `defaults.cli_args` for performance experiments:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --ignore-default-cli-args
```

This is useful when `smoke.json` pins conservative defaults such as `--disable-multiprocessing` but you want to inspect normal Rayon sheet/file parallelism.

Run Excel-backed value validation:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --excel-recalc-check
```

This opens each shrunk workbook in Microsoft Excel, forces a full formula recalculation, saves a copy, and compares worksheet cell values from the shrunk workbook to the Excel-saved copy. If Excel changes values, the runner also saves a recalculated copy of the original workbook and uses only the changed sheets to classify volatile formulas such as random or date functions. Non-volatile differences fail the case.

Run Excel screenshot validation:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --excel-screenshot-check
```

The runner first compares stored cell values. It then reads row heights from both workbooks through the Office add-in. If a row height differs, it captures the affected row for the report and skips full image comparison on that sheet. Sheets with equal row heights receive a paired pixel comparison. Full image pairs are transferred in memory and released after comparison; only difference evidence is saved. Use `--keep-screenshots` to retain full captures. The case output contains a single scrollable `report.html` covering every sheet, with focused original, shrunk, and difference images where applicable. This check requires Windows Excel and the Windows .NET 8 SDK.

Generated and preserved XLSX artifacts are deduplicated under `.tmp/workbooks/shared/<workbook-hash>/`. The visible workbook paths in regression case directories, Excel automation work directories, and recalc output directories are recreated as file links to those shared workbooks after each write. Mutable repair inputs are the exception: they are temporary real copies while being repaired, then removed after the repaired result has been written back through the shared workbook store.

The visual profile is derived from cleanup and visualization-relevant CLI options. Known profiles such as `default`, `remove-basic-textless-representatives`, `maximum-cleaning`, and `keep-all-inactive-protection-cells` get their own folders. Other combinations use `custom/<stable-config-hash>`.

Diagnose visual changes with targeted profiles:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --diagnose-visual-changes
```

The runner starts with `maximum-cleaning` for each workbook. An unchanged workbook needs no further profiles. When a sheet changes, `default` checks that sheet. Sheets that also change with `default` receive the conservative profiles; sheets unchanged with `default` receive the aggressive intermediate profiles. Values and Excel recalculation remain part of each run. Baseline cleanup flags are ignored so every profile uses its exact configuration.

All workbooks and profile results appear in one scrollable `report.html` under `.tmp/regression/smoke-visual-diagnosis-<timestamp>/`. It updates during the run and opens automatically. Each workbook and each visual difference has a stable link. Preserve the report directory and its evidence images when sharing those links. The adjacent `diagnosis.json` records completed profiles, selected sheets, and failures.

`--all-visual-profile-checks` remains an alias for this diagnosis. `--all-visual-profiles` runs the complete profile matrix for cleanup measurements without forcing Excel checks; combining it with `--excel-screenshot-check` selects the targeted diagnosis.

### ZIP archives from URLs

```bash
cargo run --release -p excel-shrink-cli -- regression archive \
  --request-file .tmp/archive-regression/request.json \
  --output-root .tmp/regression/archive-run
```

The local request file contains `{"urls":["https://example.org/workbooks.zip"]}`.
An optional `access_url` visits a private sharing link first to obtain download cookies.
Alternatively, repeat `--url` on the command line. Keep private URLs and request files
outside version control.

ZIP downloads remain under `.tmp/archive-regression/downloads` for reuse. One XLSX is
extracted at a time and checked with the targeted profiles above. Its original, cleaned
copies, recalculation artifacts and workbook caches are deleted after the check. Only
report metadata, logs and referenced difference images remain in the output directory.
The common `report.html` opens automatically and updates as workbooks finish. Files
that cannot be checked are listed separately with their failure reasons.

`--max-success-per-archive 1` performs a short trial. Visual or value differences count
as completed checks; technical failures do not count towards that limit. By default a
workbook has 1,800 seconds for all profiles, with 120 seconds for opening Excel or
starting the add-in. Adjust these with `--workbook-timeout-seconds` and
`--startup-timeout-seconds`. Individual XLSX entries larger than 2 GiB are skipped;
`--max-workbook-bytes` changes that limit.

Reuse the same output directory to resume after interruption or a limited trial.
Completed entries are not repeated. Add `--retry-skipped` to retry problem files.
The final exit status is nonzero if any files were skipped or any checks found
differences; either condition is recorded without stopping the remaining workbooks.


### Named corpus study

```sh
cargo run --release -p excel-shrink-cli -- regression study --request-file .tmp/study-request.json --output-root .tmp/regression/study
```

The request file gives each archive a display name:

```json
{
  "corpora": [
    { "name": "Corpus A", "url": "https://example.org/a.zip" },
    { "name": "Corpus B", "url": "C:/datasets/b.zip" }
  ]
}
```

Alternatively, repeat `--corpus "Corpus A" "https://example.org/a.zip"`.
The optional `access_url` works as described above. Actual dataset URLs belong in
local request files, outside version control. Both Windows and WSL can run the study;
Windows Excel and the capture host prerequisites are required.

For each corpus, preparation finishes before visual comparisons start:

1. Extract one XLSX or XLS. Excel converts legacy XLS to XLSX; conversion metadata
   records whether macros were removed. Comparisons use the converted XLSX baseline.
2. Shrink with `default`, `maximum-cleaning`, and `too-aggressive-cleaning`, sequentially.
   Shrink timings cover the cleanup and output write, excluding Excel and report work.
   Store the baseline and outputs in one disposable ZIP bundle per workbook.
3. Compare `too-aggressive-cleaning` against the baseline. Only its visually changed
   sheets are compared with `maximum-cleaning`. Only sheets still changed then are
   compared with `default`. Other profiles are not run. Uncompared profiles are labeled
   accordingly; their size and shrink measurements remain available.
4. Delete each bundle after a completed comparison. Keep difference images and records.
   Failed comparisons retain their bundle for a retry; loose workbooks are removed.

The default and maximum profiles also receive fresh Excel opening measurements.
These measure `Workbooks.Open` with manual calculation, not Excel startup or a full
recalculation. RAM is the process working set immediately after opening. Conversion,
shrinking, and visual checks remain separate measurements. Close unrelated CPU-heavy
applications when gathering timings for publication.

The dashboard is `report.html`, with corpus pages and stable workbook/difference links.
Unchanged workbooks appear in a collapsed measurement table. `verified-unchanged.csv`
is a readable, portable list of completed unchanged checks. `records/` remains the
resume journal; the runner never parses HTML to decide what to skip.

To keep earlier results, add `--import-report path/to/diagnosis.json`, or
`--verified path/to/verified-unchanged.csv`. Completed unchanged maximum/stronger checks
are retained without repeating them; missing historical timings remain missing.
Previous differences are scheduled first; untouched entries follow. Earlier technical
failures stay in the skipped table until `--retry-skipped` is requested. Reuse the same
output directory to resume.
Prepared bundles are reused only with the same executable and source archive catalog.
`--retry-skipped` revisits technical failures. A changed build regenerates pending bundles.

Use `--limit 3` for at most three new workbooks per corpus, `--prepare-only` to stop
before comparisons, `--compare-only` to use existing preparations, and `--report-only`
to import/rebuild reports without running Excel or downloading archives. As with archive
runs, differences or skipped files produce a nonzero final status while allowing the
remaining files to finish. Workbooks have a 1,800-second phase deadline and a 120-second
Excel startup deadline by default. The worker also stops if its parent disappears.

Capture windows use a separate inactive Windows desktop. Each owned Excel job has a
4 GiB private-memory limit; `EXCEL_SHRINK_EXCEL_MEMORY_MIB` can change it. Captures remain
serial. Row-height and image checks share the same two Excel sessions, with 32-megapixel
image tiles and the existing 16-megapixel fallback.

## VS Code debugging

The corpus-backed smoke regressions are also available as Rust tests inside `excel-shrink-cli`.
They are normal tests and run as part of `cargo test`.

Example:

```bash
cargo test -p excel-shrink-cli --bin excel-shrink-cli corpus_regression_smoke_suite -- --nocapture
```

Test entry points:

- `corpus_regression_smoke_suite`
- `corpus_regression_selected_smoke_case`

The selected-case test is ignored by default and uses `EXCEL_SHRINK_CORPUS_REGRESSION_CASE_ID=<case-id>`.
Set `EXCEL_SHRINK_CORPUS_REGRESSION_EXCEL_RECALC_CHECK=1` or `EXCEL_SHRINK_CORPUS_REGRESSION_EXCEL_SCREENSHOT_CHECK=1` to enable the Excel-backed checks from those Rust test entry points.

The VS Code launch configs in `.vscode/launch.json` are wired to those tests so you can debug:

- the smoke suite
- one category case chosen by case ID

## Policy

The runner intentionally does not update baselines.

It only:

- runs the CLI
- compares actual results to versioned expectations
- writes a JSON report under `.tmp/reports/<visual-profile>/`
- writes profile metadata under `.tmp/reports/<visual-profile>/` and `.tmp/baselines/<visual-profile>/`

If a baseline needs to change, do that as an explicit reviewed code change with a stated reason. That prevents real regressions from being normalized away.

### Excel process and outputs

Excel opens temporary workbook copies; the source workbooks are not moved. The
capture runner removes its temporary files and closes its Excel instance when the
run ends. Inspect the case's `report.html` for sheet results and its JSON result
for machine-readable details. Recalculation output and logs remain separate from
the visual report.
