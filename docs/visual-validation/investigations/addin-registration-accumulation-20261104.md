# Add-in registration accumulation, 2026-11-04

## Follow-up inventory on 2026-11-09

A read-only inventory of the current user's WEF Developer key found 617
registrations. File existence checks found 609 missing manifest paths and eight
existing manifests. Missing-path samples referred to old `es-office-capture-*`
directories under the user's C: temporary directory. Existing entries included
the live Destatis resident sessions and two retained diagnostic sessions.
Existence alone does not establish whether a registration is still in use.
No values were removed during this inventory.

The earlier reversible startup experiment in
[the add-in startup investigation](addin-startup-20261108.md) remained fast after
the missing registrations were restored. Accumulation is established, but its
causal contribution to current startup latency remains unproven. The existing
cleanup command requires Excel to be closed. Keep that gate when applying it.
Resident cleanup restores registrations on normal exit, while forced launcher
termination can bypass its `finally` block. This observation does not identify
which exit path produced each of the 609 missing-path entries.

The user subsequently authorized removal during the active run. Two backed-up
passes removed 567 missing capture manifests and 40 further missing capture or
legacy parallel-test manifests. Ten existing manifests remained. The live
Destatis summary advanced to 1,095 completed comparisons with four active groups.
These observations establish continued operation, not a measured speedup.

Automatic missing-only cleanup now runs immediately before `Register-AddIn`.
It considers GUID values under the current user's local temporary directory,
the process temporary directory and the current capture staging parent. Only
the known capture-manifest or legacy parallel-test filename patterns qualify.
`GetAttributes` must report file or directory absence. Access errors are reported
as cleanup warnings. A flushed JSON backup precedes deletion, and both the
unchanged registry value and missing file are checked again before deletion.
Existing manifests are preserved even when their owning process may have ended.
The older manual cleanup command retains its closed-Excel gate because it can
also remove existing abandoned manifests.

A small real Data.gov Original/Default pair timed out after 120 seconds without
an Office.js heartbeat. Excel consumed approximately one CPU core while no
WebView appeared. Visible inspection showed the workbook grid without an add-in.

The current user's WEF Developer key contained 3,085 registrations. Of these,
3,035 matched this runner's exact temporary manifest convention, including
2,893 missing manifests. The remaining 50 registrations were left untouched.
With Excel stopped, the owned entries were backed up and removed using
`tools/cleanup-capture-registrations.ps1 -Apply -BackupPath <new-file>`.

The same pair then completed in 22.983 seconds. The additional Check Performance
Optimize All comparison completed in 39.945 seconds. All resident rebuilds
retained their process identities. Both tests found no image differences and
left 50 registrations. These small samples identify a startup failure mechanism;
they do not establish corpus-wide throughput.

The host error destructor previously killed the PowerShell process immediately,
preventing resident launchers from restoring their registrations. It now requests
resident shutdown and waits at most ten seconds for launcher receipts before
forcing host termination. Normal close receipts make this wait immediate.

The maintenance script inventories by default, refuses removal while Excel is
running, verifies exact owned paths and IDs, backs up before removal, and compares
the registry value again immediately before deletion. Other developer add-ins
are outside its scope.

A deliberate native failure test delayed the host's recalculation reply for 60
seconds with a 25-second command deadline. The comparison failed with the expected
operation-specific error. Both resident launchers published cancellation receipts,
all owned Excel processes exited, and the developer registration count returned
to 50. This verifies the bounded destructor path, not every possible machine crash.
