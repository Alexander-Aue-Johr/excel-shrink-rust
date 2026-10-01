# Excel regression validation

The regression tooling validates Excel Shrink without making Excel part of the
shrinking engine. Repository-local transient state lives below `.tmp/`; persistent
baselines, reports, and measurement summaries remain below `regression/`.

## Recalculation validation

Recalculation is deliberately shrunk-first:

1. Compare the repaired baseline with the direct shrink output using the borrowed
   SpreadsheetML value/formula comparator.
2. Open only the shrunk workbook in Excel, run one blocking
   `CalculateFullRebuild()`, save a separate recalculated copy, and compare its
   stored values with the direct shrink output.
3. If no value changed, stop. The large baseline is not opened in Excel.
4. Only if the shrunk workbook changed during recalculation, recalculate the paired
   baseline and compare the freshly recalculated workbooks on the affected sheets.

The PowerShell script contains no `CalculationState` polling loop, sleep loop, or
heartbeat protocol. Rust owns timeout handling and wall-clock diagnostics. Excel is
visible during recalculation so a developer can distinguish slow work from an
interactive Excel failure.

`EXCEL_SHRINK_RECALC_TIMEOUT_SECONDS` controls the Rust-side timeout for the blocking
recalculation call. `EXCEL_SHRINK_AUTOMATION_STALL_SECONDS` controls non-recalculation
automation stalls.

## Screenshot validation and cache

Screenshots compare the repaired stored baseline with the direct shrink output; they
never use Excel-saved recalculation copies. Capture opens workbooks read-only with
manual calculation, events/macros disabled, and external-link updates disabled.

The screenshot cache is content- and environment-qualified. Ordinary external-link
metadata no longer disables reuse by itself when Excel is explicitly told not to
update links. Volatile/refresh-on-open inputs and external visual relationships such
as linked images/OLE/control content remain non-cacheable. Conditional formatting,
charts, drawings, and other contextual sheets still receive broader package context.

Current shared screenshot cache entries use the v10 namespace. Changing the guarded
render-input rules therefore cannot silently reuse older v9 cache entries.

## Screenshot capture hardening

Rust-to-PowerShell JSON is an explicit UTF-8 interface. Cache-plan, range-limit, and
skip-sheet JSON are read with `-Encoding UTF8`; the native fixture includes the names
`Inhaltsübersicht` and `Erläuterung zu CSV-Tabellen`.

Drawing bounds remain OOXML-first. Known bounds-neutral metadata extensions used for
`a16:creationId`, `a14:hiddenFill`, and `a14:hiddenLine` do not force otherwise simple
anchors through per-shape COM. Unknown extensions and unsupported geometry still
fall back to Excel. Duplicate OOXML shape names are tracked so fallback does not
silently collapse shapes into one `Shapes.Item(name)` lookup.

After three consecutive `Range.CopyPicture` exceptions, screenshot capture bisects
the range along the physically longer direction and retries recursively. Memory-like
image failures are also splittable. Clipboard-provenance and PNG/file-system failures
remain explicit failures instead of being hidden by tiling. The split depth is high
enough to cover Excel's maximum worksheet dimensions.

The renderer key already hashes the screenshot/bounds/clipboard/planner sources, so
these changes select a new cache identity within the v10 namespace automatically.

See [`REAL_WORLD_VISUAL_CASES.md`](REAL_WORLD_VISUAL_CASES.md) for research-worthy
examples and future regression fixtures.

## Logs

Every Excel Shrink process writes a fresh log instead of appending forever to one
`excel-shrink-run.log`:

```text
logs/runs/excel-shrink-run-<unix-ms>-<pid>.log
```

A visual-profile matrix is one process and therefore gets one run log, which makes it
easy to attach or inspect exactly one run. Visual timing files remain separate:

```text
logs/visual-timing-<timestamp>-<pid>.jsonl
logs/visual-slow-<timestamp>-<pid>.jsonl
```

Example:

```console
python tools/visual-tests/summarize_visual_logs.py logs/runs/excel-shrink-run-<run>.log --min-seconds 5 --top 30
```

## Generated state

Normal regression outputs now live under `.tmp/regression/`; the legacy
`regression/tmp/` location is no longer used. Validation caches live under
`.tmp/regression-cache/` and screenshot caches under `.tmp/screenshots/`.

The helper script can remove generated state without touching corpus files,
regression baselines, reports, or measurements:

```console
python3 tools/clean_generated.py --cache-only
python3 tools/clean_generated.py --fresh
```

`--cache-only` removes `.tmp/` and the legacy `regression/tmp/` directory if it is
still present. `--fresh` additionally removes `target/` and `logs/`, approximating a
fresh local checkout while preserving source-controlled research artifacts.

Equivalent VS Code tasks are installed as:

- `Excel Shrink: Clear validation caches`
- `Excel Shrink: Reset local generated state`

## Local verification

```console
cargo fmt --all -- --check
cargo check -p excel-shrink-cli --bin excel-shrink-cli
cargo test -p excel-shrink-cli --bin excel-shrink-cli regression::excel_automation -- --nocapture
cargo test -p excel-shrink-cli --bin excel-shrink-cli regression::xlsx_value_compare -- --nocapture
cargo test -p excel-shrink-cli --bin excel-shrink-cli visual_ -- --nocapture
```

PowerShell parser/native fixture checks:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\visual-tests\verify_visual_fix.ps1
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File tools\visual-tests\verify_visual_fix.ps1 -LiveExcel
```

The non-live run covers the OOXML drawing planner, UTF-8 cache boundary, split-error
classification, and PowerShell parsing. `-LiveExcel` additionally runs interactive
Excel rendering fixtures.
