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
It also retains the exact input as `baseline.xlsx` with a relative hash/length
receipt. Excel opens this verified private copy. The external source location
can subsequently move without losing the input bytes needed for transfer.
The working copy is created with SaveAs, then saved before optimization. Both
optimization controls come from the same working filename. No explicit rebuild
is requested by this operation. Source files are opened read-only with macros
and link updates disabled. The manifest records source/artifact hashes, native
confirmation, Excel identity, machine attribution, measurements and recipe hashes.
`preparation.json` retains the complete input outcome list, including individual
failures and a coordinator error if one occurred. An unsuccessful input makes the
command exit unsuccessfully while preserving other inputs' completed artifacts.

The entry point accepts an explicit input list or `--source-manifest` with stable
source occurrences and expected baseline hashes. Corpus selection, portable
package import and the preparation UI are separate integration work.
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

Attempt `06-manifest` exercised the ordinary CLI with a one-input source manifest
using the fixture corpus's existing source ID, raw-source reference and a relative
baseline path. Native optimization completed, and the output preserved those
identities and its verified baseline SHA-256. `manifest-acceptance.json` records
the checks. Focused tests cover missing paths, duplicate occurrences, selection
order, unsupported versions, relative parent paths, Windows UTF-8 BOMs and a
changed baseline failing before Excel leasing.

The `07-baselines` repeatable native test passed in 39.09 s after adding the
retained baseline. It checked both baseline copies against their recorded input
hashes, two successful jobs in one reused Excel and confirmed process exit.

Two repeatable native tests also passed. The existing conversion test exercised
two reused Excel instances, password rejection, failed-instance replacement,
scale-down, value preservation and process exit in 27.41 s. The new preparation
test repeated two confirmed jobs in one Excel process, retained controls and
checked owned process exit in 34.31 s.

## Portable export acceptance

The completed `07-baselines` preparation was exported with the ordinary CLI.

```powershell
deltasieve preparation-export PREPARATION_ROOT NEW_PACKAGE_ROOT
```

The package contained two successful occurrences, six byte-deduplicated XLSX
blobs and one producing-environment record. An independent Python sqlite3 and
hashlib check verified SQLite integrity, the database hash and every blob's hash
and length. This export required no Excel operation. Four focused unit tests
also cover duplicate bytes, failed outcomes, corrupt controls, immutable output
destinations and older receipts with an available verified external baseline.
Import preview and paired-folder materialization remain separate work.

The ordinary `preparation-verify` command subsequently accepted this native
package without Excel. Seven package tests passed, including relocation after
moving the source directory away, incomplete publication, changed header counts,
same-length blob corruption, database corruption and an invalid generation
binding even after updating the database hash in the header. Verification opens
the database read-only and checks all state and provenance references.

The source-manifest preparation from attempt `06-manifest` was also exported
and inspected through the ordinary `preparation-preview` CLI. Its exact source
occurrence, raw-source reference and local baseline matched one complete
generation. `coverage-acceptance.json` retains that result beside the private
experiment artifacts. Ten package tests passed after adding full-selection
accounting with paged details, missing and failed sources, source/baseline
mismatches, changed or missing local inputs and ambiguous compatible generations.

Strict import subsequently passed twelve package tests, including exact paired
bytes, collision-safe source-derived names, preserved producer measurements,
ordinary configuration/source discovery, immutable destinations and preflight
rejection of incomplete selections. Commit `a868510` was secured before the
native comparison acceptance run.

The ordinary CLI imported attempt `06-manifest` into `import-native-06`. Its
generated two-source configuration completed through the normal DeltaSieve run
entry point, with local optimization reported as not requested. Five regions
were captured in each rebuilt state. One region on `Wind LLC #259` retained a
geometry/pixel difference and remains unclassified; this acceptance run proves
the imported execution path, not visual equivalence or CP causation. The native
report and images remain under that import's private report directory.
The bounded window took approximately 46.26 seconds including admission wait,
and the harness restored Enron's four-group target in its finally block.

The generated import index retains the original source identity and producer
measurements. The standard comparison currently uses the safe paired filename
as its display name in that first acceptance run. Subsequent identity-index
binding adds original display names to static report rows while preserving
technical pairing and reuse keys.

Explicit generation selection and packaged-baseline adoption subsequently passed
fifteen package tests. Different baseline bytes require matching raw-source
provenance; missing or mismatched provenance remains rejected. Tests also cover
unavailable local baselines, unknown or out-of-selection generations, UTF-8 BOM
choice files, unsupported choice versions and unchanged local source bytes.

A file-only CLI acceptance run used the native package with its Saved-Original
control as an intentionally different consumer baseline. Strict import rejected
the mismatch without creating output. Import with an explicit generation and
packaged-baseline adoption succeeded, retaining the requested and adopted hashes
in `consumer-adopted-06/import.json`. This simulates differing consumer baseline
bytes on this machine; it is not a second-machine conversion experiment.

Sixteen package tests and three static-site tests passed after report binding.
The ordinary comparison entry point was exercised with a same-length modified
optimized input. It published the original source ID, display name and producer
provenance with the explicit hash-mismatch failure before creating validation
launcher logs. The static-site test verifies original-name selection and retained
producer metrics. This update did not repeat native rendering. Preparation web
controls and richer metric presentation remain separate integration work.

Package transfer now has a dedicated local web workflow. Four room tests passed,
covering operation reservations, failure release, request fields, origin checks
and bounded status behavior. Two package-control DOM/data tests cover explicit
adoption, request paths, paging restoration, invalidated form state and separate
configuration review. The existing two study-form tests also passed.

An isolated server on port 18886 exercised the native fixture's package through
the ordinary HTTP endpoints. Coverage reported one complete occurrence; import
and export each retained one occurrence. The returned comparison configuration
included its imported identity index. A tokenless POST was rejected, the module
was served, and the room returned to inactive afterward. The owned test server
was stopped. `package-web-acceptance.json` retains this private receipt. This
acceptance did not launch Excel, change the production Enron host or visually
inspect the web layout. Browser layout acceptance and native preparation-job
controls remain open.

The preparation manager now emits one compact progress snapshot per second,
plus initial and terminal snapshots. Four focused non-native preparation tests
passed, covering queued/in-flight/terminal accounting, worker-count validation,
legacy request compatibility and expected-baseline rejection before Excel lease.
The existing ignored native test additionally asserts the completed progress
receipt; it was not rerun for this telemetry-only change. The background web-job
start/status integration remains open.

Commit `73e3a8b` adds independent background preparation start through the web
configurator. Six backend tests and four JavaScript tests passed, including
frozen selection paths, terminal failure receipts, invalid concurrency/output
rejection, explicit controls and compact status rendering.

The bounded native HTTP acceptance test submitted the one-member source manifest
with two requested workers, with actual admission limited to its one input. The
start response returned in approximately 0.013 seconds and native preparation
completed in approximately 22.11 seconds. A second start during the job was
rejected. Status polling observed in-flight progress, then a completed job with
confirmed optimization, no failed inputs and a stopped pool. The owned Excel PID
was absent afterward and the source workbook hash remained unchanged. These are
functional observations under concurrent Enron load, not a throughput benchmark.

`native-web-job-acceptance.json`, `native-web-job-integrity.json`, the frozen
selection/job receipts and the exact producer binary are retained beside the
private experiment data. Enron admission was restored to four in a finally block
and the isolated test host was closed. Browser layout acceptance, owner-restart
recovery and live target adjustment remain open.

## Repeatable native tests

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
