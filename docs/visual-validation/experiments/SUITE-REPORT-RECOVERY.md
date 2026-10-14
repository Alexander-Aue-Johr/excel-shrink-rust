# Suite report recovery and long-path IO

## Incident
The 20261013T150941508Z-611ba474 suite finished 16 capture workloads (four
warm-ups and twelve measured trials). Post-processing did not finish:
Windows PowerShell 5.1 directory enumeration failed on nested WSL/UNC paths,
and its file-existence probe reported every measured final PNG as missing.
The unfinished metadata ZIP has 28 readable entries but only one case subtree
and no bundle-manifest.json. A valid ZIP container is not a complete review.

## Repair existing evidence
Apply the report-IO patch, then run the VS Code task:
`Visual validation: Repair latest suite reports (no Excel)`

The task rebuilds each case report, optionally rehashes existing final PNGs,
and publishes a verified metadata archive. It does not start Excel, change
PNGs or raw capture JSON, or repeat a timed workload. Historic workload_ms and
recorded suite completion time are preserved. Previous metadata is backed up
outside the run, under .tmp/visual-validation/report-repair/.
Deleted PNGs cannot be recreated by this task and remain unverified.

## Implementation
A small report-only C# helper uses Unicode Win32 IO with extended UNC paths.
No registry edits, drive mappings, certificates, new packages or WPF dependency
are introduced. Access failures and absent files are distinguished.
PNG decoding uses a stream, not GDI+'s filename loader.
Both canonical digest identifiers are recognized; different algorithms are
never compared as equal merely because their digest strings match.

Bundle publication has a separate status. All expected files are enumerated
before writing a uniquely named temporary ZIP. A completion manifest is
written last. Before publishing, every entry is read back and checked against
its SHA-256 and uncompressed length, and every completed case's metrics must
be present. A missing final archive is not linked as available.
Old .new files are retained as evidence, not blindly renamed or promoted.

## Scope
Capture policies, budgets, transports and timing boundaries are unchanged.
This addresses report discovery, offline verification, and bundle publication.
It is not a benchmark of alternative screenshot implementations.

## Tests
Node tests cover both digest identifiers, incompatible digest versions, and
publication links. The Windows-only read-only smoke test exercises native enumeration and PNG
decoding of a user-selected long path. Archive publication itself still requires
native integration testing. Windows-specific tests must be run on Windows PowerShell 5.1.
