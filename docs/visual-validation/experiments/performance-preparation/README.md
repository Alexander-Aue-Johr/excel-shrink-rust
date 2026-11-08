# Persistent Check Performance preparation

This experiment checks independent artifact generation, instance reuse,
continuous refill, failure isolation and owned cleanup. It does not measure
candidate-selection accuracy or certify visual equality.

## Command and retained states

```powershell
deltasieve regression performance-prepare `
  --input 'G:\fixtures\one.xlsx' 'G:\fixtures\two.xlsx' `
  --output 'G:\experiments\new-preparation' --workers 2
```

Use a new output directory and prepared XLSX inputs. Worker count is independent
of the shrinker thread count and can exceed logical processors. Admission keeps
at least 512 MiB of available physical/commit memory. Each worker owns its Excel
process and private desktop. A completed request closes its workbook before the
same worker accepts another. A native failure retires that worker and allows its
replacement and other workers to continue the queue.

Each successful input retains `original-saved.xlsx`,
`before-optimization.xlsx`, `after-optimization.xlsx` and `prepared.json`.
The working copy is created with SaveAs, then saved before optimization. Both
optimization controls come from the same working filename. No explicit rebuild
is requested by this operation. Source files are opened read-only with macros
and link updates disabled. The manifest records source/artifact hashes, native
confirmation, Excel identity, machine attribution, measurements and recipe hashes.
`preparation.json` retains the complete input outcome list, including individual
failures and a coordinator error if one occurred. An unsuccessful input makes the
command exit unsuccessfully while preserving other inputs' completed artifacts.

The entry point currently accepts an explicit input list. Corpus selection,
portable package import and the preparation UI are separate integration work.
These files are generation evidence, with visual validation still pending.

## Native observations on 2026-11-08

Working receipts are under
`G:\deltasieve-work\performance-pool-native-20261108`.
The valid input is the frozen Enron reference member
`0966d3e126908018b94a`. Excel version was `16.0.20527.20150`.

| Attempt | Workload | Outcome |
| --- | --- | --- |
| 01 | Two inputs, one worker | Failed. The default-desktop helper could not inspect the private desktop window. |
| 02 | Same workload | Failed. Moving the already initialized PowerShell thread to another desktop returned resource-in-use. |
| 03 | Helper started on the private desktop | Failed startup. Get-FileHash was unavailable in that helper. Native .NET stream hashing replaced that dependency. |
| 04 | Two valid inputs, one worker | Two confirmed optimizations, same Excel PID 46940, completed-job counters 1 and 2, three retained controls each. Approximately 35.01 s. |
| 05 | Five valid inputs and one invalid package, two workers | Five confirmed optimizations and one expected native open failure. Failed PID 3804 was replaced by PID 36052. Peer PID 41684 completed three inputs. Approximately 61.59 s. |

The invalid package contains `xl/workbook.xml` with invalid content. It passes
the deliberately shallow input-envelope check and exercises the native failure
path. Its failure remains in the six-input denominator and produces exit code 1.
Worker shutdown receipts report zero remaining processes. The native Rust test
additionally checks retained PID/creation-time identities after teardown.
These timings include concurrent Enron activity and are correctness observations,
not a controlled throughput comparison.

Two repeatable native tests also passed. The existing conversion test exercised
two reused Excel instances, password rejection, failed-instance replacement,
scale-down, value preservation and process exit in 27.41 s. The new preparation
test repeated two confirmed jobs in one Excel process, retained controls and
checked owned process exit in 34.31 s.

## Repeatable tests

Set TEMP, TMP, EXCEL_SHRINK_LOG_DIR and EXCEL_SHRINK_VISUAL_LOG_DIR to a working
drive before running these tests. Set PERFORMANCE_PREPARATION_FIXTURE to an XLSX
fixture and LEGACY_CONVERSION_FIXTURES to a directory containing normal.xls and
password.xls. Run the native tests serially with a bounded external deadline.

```powershell
cargo test -p excel-shrink-cli --lib `
  native_performance_worker_reuses_excel_and_retains_controls `
  -- --ignored --nocapture --test-threads=1

cargo test -p excel-shrink-cli --lib `
  native_pool_reuses_excel_and_recovers_failed_sessions `
  -- --ignored --nocapture --test-threads=1
```

The experiment temporarily reduced Enron admission to three groups for each
native window and restored four in a finally block. Do not infer large-corpus
throughput or safety for every Excel prompt from this bounded fixture set.
