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

## Visual validation

The visual check compares the repaired stored baseline with the direct shrink output;
it does not use recalculation-saved copies. Stored values are checked first. The
Office add-in then reads row heights from both workbooks. A changed height leads to
a focused capture of the affected row. If all heights match, the add-in
captures worksheet images and Rust compares their pixels. The scrollable case
report presents each affected sheet with its original and shrunk image excerpts.

Drawing bounds come from the workbook package and are checked against Excel's
rendered geometry during capture. An unresolved capture is an error, not a match.

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

Normal regression outputs live under `.tmp/regression/`. Validation caches live
under `.tmp/regression-cache/`.

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

Unattended conversion and recalculation pass empty open/write passwords and ignore
read-only recommendations. Encrypted workbooks fail through the existing automation
error path instead of waiting for a password dialog; readable workbooks with write
reservations still open read-only.

On Windows with desktop Excel installed, run the opt-in integration test:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File excel-shrink-cli/office-capture/tests/automation-passwords.tests.ps1 -Run
```

It creates synthetic XLS/XLSX fixtures for normal, read-only recommended,
write-reserved, and encrypted workbooks, then runs the production conversion and
recalculation scripts. Each operation has a 45-second deadline. It checks saved
values/formulas and unchanged source files, and retains fixtures and logs under
`.tmp/automation-passwords-<guid>/`.

```console
cargo fmt --all -- --check
cargo check -p excel-shrink-cli --bin excel-shrink-cli
cargo test -p excel-shrink-cli --lib regression:: -- --nocapture
cargo test -p excel-shrink-cli --lib regression::xlsx_value_compare -- --nocapture
cargo test -p excel-shrink-cli --lib visual_ -- --nocapture
```
