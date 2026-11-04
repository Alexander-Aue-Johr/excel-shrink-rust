# Add-in registration accumulation, 2026-11-04

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
