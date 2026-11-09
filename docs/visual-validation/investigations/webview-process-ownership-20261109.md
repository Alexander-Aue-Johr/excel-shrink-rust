# Live WebView2 ownership inventory, 2026-11-09

The user reported many WebView2 processes and asked whether indirect children
of the validation pipeline had become orphaned. A read-only CIM inventory
recorded process IDs, creation times, parent identities, WebView roles and
profile directories. Each WebView parent chain was followed to a non-WebView
owner, rejecting absent parents, cycles and a reused parent PID whose creation
time was later than its child's.

The snapshot contained 55 `msedgewebview2.exe` processes. Forty-eight resolved
to eight live Excel owners, six WebView processes per Excel owner. The eight
browser-root profile paths belonged to the current Destatis warm-worker pool.
The other seven processes resolved to the Windows SearchHost owner. None of
the observed chains had a missing or reused parent identity.

The first inventory's combined working sets totaled about 3.56 GiB across all
55 WebViews, including Windows Search. This is not a private-memory or physical
memory-pressure measurement. A subsequent three-second CPU sample matched
process ID and creation time before calculating deltas. All 55 process identities
survived that interval. On sixteen logical processors their combined CPU use was
about 0.68 percent. This short sample does not exclude intermittent CPU, GPU,
storage or memory effects and does not diagnose the other machines.

The raw inventory is retained privately at
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp\webview-inventory-20261109T131821.json`.
No process was terminated by this inspection.

The source ownership audit found kill-on-job-close ownership in both
`process_job::Job` and the resident C# job wrapper. The warm pool gates its start
until assignment to its owned job. The resident wrapper also provides explicit
job termination and empty-job confirmation. This is code inspection alongside
the live inventory, not a new end-to-end crash cleanup test. Shared Windows or
Office process ownership must remain distinct from a merely absent parent PID.
