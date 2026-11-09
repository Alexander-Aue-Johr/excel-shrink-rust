# Destatis full-grid geometry expansion

The four-group night run exposed three workbooks with continuously successful
geometry measurements but no completed image captures. Their retained plans
contain literal `A1:XFD1048576` rectangles. This is the entire 17,179,869,184-cell
Excel grid. Geometry progress alone kept their idle watchdogs alive.

- Group `e4411742c9c5bb6d9b0d`, `eheschliessungen-geborene-gestorbene-5126001217005.xlsx`,
  requests the entire `Jahr` sheet. The rebuilt original declares `A1:T617` and
  contains 10,983 serialized cells. Its partner declares `A1:S617` with 9,268.
  Neither state has CF on that worksheet. Inherited solid fills on rows and
  columns expand the current content envelope in both directions.
- Group `0c9940f412911ccacfe7` requests complete grids on `Ost VZ ` and `West VZ `.
  The retained rebuilt worksheet dimensions are finite. Column and row formats
  likewise contribute unbounded inherited visual extents.
- Group `3bf889f907d5bef2c265`, `baumschulerhebung-2030317219005.xlsx`, includes many
  full-grid areas. Some worksheets have almost-grid-wide CF application areas.
  The inspected rule compares a cell with the literal text `"."`. Other rules
  and inherited extents still need separate analysis.

The relevant original and rebuilt copies, plan indexes and telemetry remain in
the ordinary night-run evidence directories. The inspection did not alter them.
At inspection time two cases had already accumulated millions of measured range
observations without reaching image capture. This is a planning/cost failure,
not evidence that those workbook pixels differ or that the files are invalid.

## Targeted launcher circuit breaker

The launcher now observes the existing atomic plan index and refuses an exact
whole-grid request with `unbounded_capture_plan` and `coverage=unverified`.
It retains the raw plan and publishes the failure through the existing durable
failure/live-state path after owned-process cleanup. It scans at most 4,096
entries when the plan index changes. This guard does not silently clip coverage,
declare equality or remove large bounded-column cases.

The pure PowerShell test covers full-grid rejection, accepted finite areas,
accepted million-row bounded-column areas and unrelated index stages. A native
small presentation comparison completed through the normal launcher in 11.708
seconds. The retained `Jahr` pair was stopped with the explicit unverified plan
failure after 81.151 seconds overall, including opening, rebuild, height work
and cleanup. The guard is reached after those earlier phases. Their cost remains
a separate optimization target. Native evidence is under
`G:\deltasieve-work\full-grid-guard-20261109`.

Further work must resolve finite representative coverage for inherited solid
backgrounds and CF outside content. The original cases remain unresolved and
must be retried when that proof is implemented. A failed plan is not a successful
visual validation result.

## Night-run deployment and recovery

The ordinary study termination API was used before resuming from the existing
launch receipt with a separately packaged asset runtime at source `6c744038`.
All 113 runtime files were verified. Production executable bytes stayed unchanged,
and the existing USB distribution source was not modified. All 1,182 prepared
inputs and 240 completed comparisons survived the transition. Subsequent API
observations showed four active groups and 273 completed comparisons, with two
skipped cases retained separately from successful validations.

The previous targeted child was briefly observed after the coordinator reported
zero groups. Later process inspection no longer found that old host-script
process. This observation does not prove the cleanup lifecycle is correct for
every descendant. Keep descendant cleanup as a separate investigation rather
than treating the coordinator's empty group counters as process-exit evidence.

The migration receipt and previous launch/configuration/control files remain in
the private night-run `runtime-migrations/6c744038` directory. They bind the asset
checkout, unchanged executable hash and previous/current launch hashes. No raw
finding or failed attempt was discarded by this recovery.
