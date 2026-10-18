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

The runner first compares stored cell values. It then reads row heights from both workbooks through the Office add-in. If a row height differs, it captures the affected row for the report and skips full image comparison for that case. If values and row heights match, it captures paired worksheet images and compares their pixels. The case output contains a single scrollable `report.html` covering every sheet, with focused original, shrunk, and difference images where applicable. This check requires Windows Excel and the Windows .NET 8 SDK.

Generated and preserved XLSX artifacts are deduplicated under `.tmp/workbooks/shared/<workbook-hash>/`. The visible workbook paths in regression case directories, Excel automation work directories, and recalc output directories are recreated as file links to those shared workbooks after each write. Mutable repair inputs are the exception: they are temporary real copies while being repaired, then removed after the repaired result has been written back through the shared workbook store.

The visual profile is derived from cleanup and visualization-relevant CLI options. Known profiles such as `default`, `remove-basic-textless-representatives`, `maximum-cleaning`, and `keep-all-inactive-protection-cells` get their own folders. Other combinations use `custom/<stable-config-hash>`.

Run every predefined visual profile with Excel value/recalculation and screenshot validation:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --all-visual-profile-checks
```

This ignores baseline-level default cleanup args for the matrix and supplies each predefined profile explicitly. Use `--all-visual-profiles` instead when you want the same profile matrix without forcing both Excel-backed checks.

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
