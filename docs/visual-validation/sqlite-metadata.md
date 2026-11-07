# SQLite metadata

Metadata migration is separate from activating a new runtime storage backend.
The migration command never deletes JSON files, changes the corpus queue, or
starts Excel. PNG and workbook evidence remains external.

## Lossless migration

Run these commands while metadata producers are stopped. A paused comparison
target alone does not freeze the runner's status publications.

```powershell
excel-shrink-cli.exe metadata-store import RUN_DIRECTORY NEW_DATABASE.sqlite
excel-shrink-cli.exe metadata-store verify RUN_DIRECTORY NEW_DATABASE.sqlite
excel-shrink-cli.exe metadata-store export NEW_DATABASE.sqlite NEW_EXPORT_DIRECTORY
```

Import retains every `.json` and `.jsonl` file as its original bytes, including
unknown fields and incomplete diagnostic lines. It records a SHA-256 checksum
and relative path for each document. Links are not followed. Image and workbook
files are not imported.

The importer writes a uniquely named temporary database beside the destination.
A second pass checks the complete source inventory and every document against
the database. SQLite's integrity check must also succeed before the verified
database is published. The destination must not already exist. A failed import
leaves the original tree untouched and does not activate the partial database.

An export requires a new directory and rejects absolute or traversing paths.
Every payload is checksum-verified before export. Import and export preserve
formatting and diagnostics rather than normalizing or interpreting them.

Creation and modification times are not preserved by JSON export. Paths,
payload bytes and checksums are retained. This command is a metadata migration,
not a complete backup of binary research evidence.

SQLite is bundled into the Windows executable. There is no additional database
service or .NET installation requirement.

## Runtime diagnostic storage

Set `EXCEL_SHRINK_SQLITE_METADATA=1` in the runner environment before launching
it. Child comparison and resident hosts inherit the setting. Existing runs and
their scientific result records remain readable. Clearing the variable restores
legacy publication for newly started comparisons.

Each comparison owns its `telemetry.sqlite`. It commits at most one transient
snapshot per second and commits the terminal state immediately. Each resident
host separately owns its session database and accepts authenticated event batches.
Event UUIDs make retried batches idempotent. The host acknowledges only after
the transaction commits. WAL mode and `synchronous=FULL` protect acknowledged
transactions when the process terminates unexpectedly.

Small compatibility references replace duplicate display snapshots. The native
readers resolve these references and checksum the payload. Live database values
are refreshed even if the reference file itself did not change. Finished values
use the bounded display cache. `metadata-store read PATH` resolves either form.

Session `metrics.json` remains a full compatibility snapshot because existing
PowerShell opening and resource observers read it directly. The comparison's
local telemetry reference retains watchdog sequence counters. These contracts
preserve readiness detection and capture timeouts.

Launcher events are batched during execution and exported to JSONL during
teardown. Unacknowledged events remain queued and are written to the legacy
journal if teardown cannot reach the host. Committed events remain in SQLite
even if the final export is interrupted. Source archives, images, workbooks,
scientific result records and command/response IPC files keep their existing
formats. This change does not consolidate every JSON file in a run.

Do not copy a live database alone as a backup. Its WAL may contain committed
data. Stop its writer or use SQLite's backup mechanism before copying.

SQLite is not a compression format. Exact-byte migration may use more logical
bytes than the JSON payloads. Reduced file creation and duplicate publication
are the intended benefits. Defender cost and throughput require measurement on
the target machine.
