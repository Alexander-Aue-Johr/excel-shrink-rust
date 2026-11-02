# Independent Check Performance generations

Native Windows/Excel verification, 2026-11-02. All Excel instances ran on owned
private desktops. Test artifacts stayed in `.tmp/performance-cache-native`.

Two runs used the same Original with a dependent formula `B2 = B1*2`. The first
Default changed B1 from 1 to 2; the second changed it to 3. This simulates changing
the shrinker result while keeping the independent CP input unchanged.

| Observation | First run | Second run |
| --- | --- | --- |
| Independent native Optimize all outcome | Confirmed `no_optimization_needed` | Restored same confirmed generation |
| CP artifact source | Newly generated | Verified cache hit |
| CPR artifact source | Explicit full rebuild | Verified cached rebuild |
| CP Optimize / CPR Recalculate telemetry | Done / done | Skipped / skipped |
| Actual comparison pairs | 9 | 9 |
| Changed Default detected | Yes | Yes |
| CP/CPR generation measurements | Recorded | Preserved exactly |
| Current cache-copy opening measurements | Generation observations | Separate new observations |

Both native reports passed the read-only consumer contract audit. The additional
deterministic-dependency gate was checked against the actual native CP parent,
as well as pure/volatile/unknown formulas and synthetic XML package cases.

These small files test correctness and reuse, not corpus throughput. The same
Excel states still need opening for current captures; tiny-file durations are
dominated by native opening/add-in startup. No runtime speedup is inferred here.

## Reproduce

Generate fixtures with the sibling
`state-selection/create-fixtures.py --output <fresh-fixtures>` (Python/openpyxl).
Use `changed-formula/inputs/original.xlsx` and `default.xlsx`, a newly built CLI,
and an existing empty group directory for each run:

```powershell
$env:EXCEL_SHRINK_CHECK_PERFORMANCE_CACHE_DIR = '<absolute-persistent-test-cache>'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File excel-shrink-cli/office-capture/run-targeted-comparison.ps1 -Binary <cli.exe> -Original <original.xlsx> -Default <default.xlsx> -Output <fresh-first-result> -ObservatoryGroup <first-group> -OperationTimeoutSeconds 180
```

Change only the Default fixture's B1 to 3 and run again with new result/group
directories, retaining Original, cache and host scripts. Do not pass
`-SkipCheckPerformance`. Inspect `check_performance_cache`, `state_decisions`,
`comparisons` and the two skipped generation steps. Both runs must still report
the changed Original/Default capture. Historical measurements must remain equal;
current opening samples must be stored separately.

Focused automated tests cover byte-content binding, narrow metadata
normalization, invalidation by saved values/styles/recipe, corruption, independent
working copies, incomplete publication, concurrent generations, dependency
eligibility and rejection of cached reports without native provenance.

The corpus launcher sets a persistent CP cache explicitly because its generic
worker cache is disposable. Existing completed corpus reports are reused without
being rerun merely to populate the new artifact cache.
