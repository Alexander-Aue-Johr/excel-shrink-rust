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

## Bounded native background countercheck

The retained `Jahr` pair was checked independently with four explicit areas,
`A1:V256`, `A257:V512`, `A513:V620` and `W1:X620`. These cover the serialized
content and nearby right/bottom background samples. They do not cover the whole
grid. Native object qualification returned no pictures or charts on this sheet
in either rebuilt state. The reference completed in 17.347844 seconds while the
night study continued with four active groups.

| Area | Original image | Shrink image | Changed overlapping RGBA pixels | Extra original pixels |
| --- | --- | --- | --- | --- |
| A1:V256 | 2348 × 5533 | 2146 × 5533 | 0 | 1,117,666 |
| A257:V512 | 2348 × 5533 | 2146 × 5533 | 0 | 1,117,666 |
| A513:V620 | 2348 × 2347 | 2146 × 2347 | 0 | 474,094 |
| W1:X620 | 407 × 13411 | 205 × 13411 | 0 | 2,709,022 |

Every extra pixel was opaque white, with all four RGBA channel extrema equal
to 255. This is sampled evidence of excess capture geometry, not a proof that
every unsampled region is equal. The failed production coverage remains unresolved.

Both worksheets disable gridlines. The inherited fills include indexed color 9
and theme color 0, and column D uses no fill. Thus the existing same-fill-ID,
full-width background proof does not apply directly. Resolving actual color
properties and the interaction with no-fill/gridline presentation is required
before broadening the proof. Merely accepting every solid fill would be unsound.

Evidence is retained under
`G:\deltasieve-work\destatis-background-probe-20261109`.
The plan SHA-256 is
`cfbfa59090e879e5a6eebd5921450716e1a83eb8de21f37e23eba0202ed21d4e`.
The input hashes recorded by the successful reference are
`dc84b83ca4fd9fde73b78f89aa42b755a985e625590fbc18a1789df0debe8417`
and `81904c78164aac3ef9e84ab212ca2cc9487f64e7b43576f5ee142e036d17b926`.
The diagnostic executable hash is
`92653fc1f36a28984d74081301632c8b0b339e07a05d536ffd854504460306b1`.

Two setup failures remain retained beside the successful `reference-matched-assets`
attempt. The first used a stale executable without the object-check argument.
The second used stale JavaScript assets without the matching operation. Rebuilding
the diagnostic executable and using the current checkout's assets resolved both.
Neither failed setup supplied visual evidence or changed production runtime files.

## Finite band plan and deliberate-change control

The new band rule avoids assuming that different fill IDs or color references
are equal. With gridlines hidden and all inherited fills flat and borderless,
the planner retains every finite row/column transition, serialized cell and a
sample of the repeating tail. CF and unsupported layouts keep the fallback.
Coverage records explicitly mark `background_tail_sampling`.

The retained rebuilt pair now selects `Jahr!A1:V619` and `Titelseite!A1:I61`
before live geometry partitioning. The XML-only inspection took 0.72 seconds
and selected 11 internal grid tiles. The normal native comparison partitioned
these into four captures per state, completed in 34.713 seconds, and found no
changed pixels. Its report hash is
`b7c09aa2ba58be8c91f1523072ee400749c6374f68e97301ded5681c9cca305f`.

A separate control changed the sole indexed-9 foreground in the original's
styles part to opaque cyan. Every other archive member retained its bytes.
The resulting input hash is
`872ce74ed8e6ca235b727390fc542db045685d9d232567620f0f082d4f745f05`.
The ordinary candidate selector and native comparison detected changed pixels
in all three selected `Jahr` regions, completing in 37.692 seconds. Its report
hash is `d1af5904c7dfea1d33b9dd3d458ac72167acf3351f4cdd14162650046bdff625`.
The unchanged title sheet needed no capture in that control. The normal attempt
first failed before Excel because its test group directory was absent. That
setup failure remains retained separately.

These bounded controls establish behavior for this rule and fixture. They do
not establish corpus-wide recall. The running study still uses its previously
frozen executable until a separately verified runtime deployment. Failed
whole-grid corpus attempts must be retried after that deployment, with their
earlier evidence preserved.

## Visible-gridline Mikrozensus tail observations

The final-results Mikrozensus workbook has a `12211-03` sheet whose retained
rebuilt content extent is `A1:E50`. Column styles extend through XFD. Style 75
has a solid indexed-9 foreground and no borders. Style 2 has no fill or borders.
The sheet view omits `showGridLines`, so its default is visible. The existing
background-band eligibility rule rejects this view and keeps whole-grid bounds.
The current CF exclusion does not help because this sheet has no CF blocks.

Two bounded same-input native references used the preserved profile original,
SHA-256 `3947034b0125e388e9c4fe08db667ac3d4395383dae29cd419ff87e15b572566`,
and producer `6cd1fa9eb0d5afb72b4bd6a8b3f2183607dcfdf535038e3c5b72a915874e80b7`.
Both copies were independently rebuilt and saved by Excel before capture.

| Areas | Image dimensions | Hash shared by all six images | Elapsed seconds |
| --- | --- | --- | ---: |
| `A804:N810`, `A10000:N10006`, `A1000000:N1000006` | 1442 by 120 | `f8b32e22382ea9c0223e152af15594386e643be0055d42cbb58f8f0e2b3a7c02` | 14.4202622 |
| `M804:Q810`, `M10000:Q10006`, `M1000000:Q1000006` | 321 by 120 | `d73cdc151672cdbfac323ccc827ec72d8fcfac8487673adf24414d6c5efabbf1` | 14.2300723 |

The second set crosses the filled/unfilled column transition. Both representative
images were visually inspected and are entirely white, including the transition
capture. Thus these retained images do not demonstrate visible gridline ink.
They establish repetition of these six supplied areas in this rendering path.
A deliberate changed-fill or changed-layout control remains necessary before
extending the production eligibility rule. No production rule was changed.

Evidence is retained under the private
`G:\deltasieve-work\gridline-tail-controls-20261109` directory. The transition
reference receipt SHA-256 is
`bc79baa49a4a18fc2a4486faaf73a0766e1cd7b1f1629d9a2f3ec5b08e58ced2`.
The initial invocation used rebuilt artifacts that normal completed-attempt
retention had already removed. It failed before launching Excel. The controls
were then run against the preserved profile original instead.

## Optimized deployment acceptance

The optimized `0ad30f53` package passed the same native fixture with four
compared regions and no differences. Its 114 packaged files were verified.
The production study was then stopped through its termination API. The runner
and queue processes exited, and no matching targeted child remained in the
process inspection. The old, independently identified Observatory host was
stopped because the resume contract rejects a different host executable hash.

The new executable was copied to its immutable hash directory. The saved
launch receipt now points to executable SHA-256
`b19fdf990c47ce532f470f2402aa4835a2e2a7d7251a57bfb379896b98920307`
and the matching packaged assets. Its arguments explicitly accept preparations
from the old producer
`1aecbdb71950abf05b1610eccc72e6fd615d71da6373260ac5feb55810a03558`.
The compatibility review found no changes to `excel-shrink-core` or
`visual_profile.rs` between the previous package's asset base `d78e9957` and
`0ad30f53`. Source identities and requested profile checks still apply.

The ordinary resume path retained all 1,182 prepared files and 360 successful
visual comparisons. It reused 822 preparations for the remaining selection,
including retries. The new runner, Observatory and four targeted children were
verified against the new executable identity. The former whole-grid `Jahr`
workbook completed its retry as unchanged, bringing the successful comparison
count to 361. Earlier failed and interrupted attempts remain retained.

The private sequential queue uses the new runtime for its subsequent Data.gov
and Enron starts. The six-machine USB source remains untouched. Migration
receipts, old launch/configuration/control files and before/stopped summaries
remain under the night run's `runtime-migrations/0ad30f53` directory.

## Entirely white far-tail captures

The later Destatis report for `arbeitskraefte-2030218209005.xlsx` includes
2,165 flagged region pairs with equal overlapping pixels and white exterior
pixels. A retained pair on `0620T!ACC1070:AGX1166` has image sizes 7,312 × 1,810
and 3,781 × 1,810. Both complete images were independently checked as opaque
white. Their native left coordinates differ, 34,473.75 versus 22,985.25 points,
so the earlier matching-origin gate retained a geometry difference.

`retained_white_pair_ignores_far_tail_geometry` exercises these actual retained
images through the production Evidence comparison. It passed with the new
complete-blank-image exception and `blank_margin_ignored=true`. The ordinary
unit control also shifts both origins and extents, and confirms that an opaque
gray image remains a visible geometry/pixel difference. The broader targeted
comparison suite passed with 156 tests and 15 ignored tests. The retained-image
test was run explicitly in addition to that suite.

The rule accepts only pairs whose complete decoded images are opaque white or
fully transparent. Equal overlapping pixels plus a white exterior do not by
themselves prove that this narrower rule applies. Uniform colored areas and
continuous borders remain separate review cases. Historical production verdicts
are preserved until explicit retrospective classification. The retained finding
input and initial per-region measurements are under
`G:\deltasieve-work\destatis-difference-triage-20261109`.
