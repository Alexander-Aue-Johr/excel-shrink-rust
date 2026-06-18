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
|- baselines/
|  |- smoke.json
|  `- corpus.json
|- reports/          # generated, gitignored
`- tmp/              # generated outputs, gitignored
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
- contains a few representative category-oriented cases
- skips slow flagship cases by default
- uses `regression/baselines/smoke.json`

`cargo run --release -p excel-shrink-cli -- regression corpus`

- intended for heavier full-corpus runs
- may contain many cases plus aggregate expectations
- uses `regression/baselines/corpus.json`

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
- `slow`
- `cli_args`
- `analysis_mode`
- `max_output_bytes`
- `min_reduction_ratio`
- `analysis_checks`

Use `relative_input` for files below the corpus root. Use `input_path` only for explicit absolute or repo-relative paths.

Use `download_url` and `cached_file_name` when the workbook should be fetched into the local gitignored cache automatically.

Use `slow: true` for heavyweight but important sentinels that should not run in the default fast smoke pass.

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

Smoke run including slow flagship cases:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --include-slow
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

## VS Code debugging

The corpus-backed smoke regressions are also available as Rust tests inside `excel-shrink-cli`.
Enable them with the `corpus-regression-tests` feature.

Example:

```bash
cargo test -p excel-shrink-cli --bin excel-shrink-cli --features corpus-regression-tests corpus_regression_default_smoke_suite -- --nocapture
```

Test entry points:

- `corpus_regression_default_smoke_suite`
- `corpus_regression_full_smoke_suite`
- `corpus_regression_selected_smoke_case`

The selected-case test uses `EXCEL_SHRINK_CORPUS_REGRESSION_CASE_ID=<case-id>`.

The VS Code launch configs in `.vscode/launch.json` are wired to those tests so you can debug:

- the default fast smoke suite
- the full smoke suite including slow cases
- one category case chosen by case ID

## Policy

The runner intentionally does not update baselines.

It only:

- runs the CLI
- compares actual results to versioned expectations
- writes a JSON report under `regression/reports/`

If a baseline needs to change, do that as an explicit reviewed code change with a stated reason. That prevents real regressions from being normalized away.
