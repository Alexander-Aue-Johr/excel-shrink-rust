# Data.gov watch findings, 2026-11-04

The validation campaign is ongoing. These observations distinguish product defects,
capture artifacts and unavailable baselines. Raw images and XML transitions remain
in the run evidence.

## Confirmed Default regression

`Fig 10_New 10-29-19_b3bff94c7378.xlsx`, sheet `Fig. 10`, centers a heading across
D2:G2 with `centerContinuous`. Default removed F2 and G2 and shifted the heading.
Restoring only G2 did not restore the layout in a native comparison. The new
content-seeded contiguous-span rule retains the necessary cells. The fixed release
output is byte-identical to this 11,668-byte input. Isolated centering formatting
without content remains removable.

An offline style inventory checked all 8,623 prepared bundles in 101.05 seconds,
without errors. It identified 144 possible affected workbooks. Their records and
old bundles were backed up before selective invalidation. Unaffected preparation
builds remain explicitly approved. Capture policy v8 also includes these spans
in the visual envelope; earlier reports retain their original coverage versions.

## Capture boundaries requiring follow-up

`DataTables2And3_313f11e1abbf.xlsx` and
`Sec4.2 CO2 Result Reasonableness Check_050b0f8ad9d0.xlsx` have matching measured
content cores but different outer context dimensions. Their images are respectively
637x268 versus 653x267 and 580x239 versus 560x239 pixels. Every pixel in each
common top-left-aligned rectangle matches exactly. Overflowing text reaches the
unequal capture edges. These observations indicate a viewport/cropping artifact;
the raw differences remain unresolved by the automated verdict. No blanket pixel
tolerance or automatic acceptance of unmatched overflow ink was added.

## Blocking compatibility dialog

The Original of
`average-expenditures-per-patient-by-healthcare-priority-group-fy2000-to-fy2017__d6c7a4a6-2939-4d64-a2c7-973910bbfc7f_57f62be347d7.xlsx`
opened and sent add-in heartbeats but its save waited for a modal `NUIDialog`
named `Microsoft Excel - Compatibility Checker`. A native repeat with the new
classification promptly produced `compatibility_confirmation_required`, including
phase, owned PID and observed title. No prompt was accepted. This is an unavailable
saved baseline, not a successful comparison or a demonstrated Shrink defect.

## Transient opening failure

`odp_hcp_flu_vaccination_by_county_2020-21_datadictionary_44db6540ef69.xlsx`
had one 180-second opening timeout. Original reached add-in readiness while Default
did not start a WebView. An isolated repeat of the exact same inputs completed.
This establishes recoverability, not a proven root cause. The main run returns to
four groups while further opening failures are investigated.

See also the [registration accumulation investigation](addin-registration-accumulation-20261104.md)
and the [blank-margin contract investigation](blank-margin-contract-20261104.md).

## Empty-row font regression

`MML Accolades winners 2022 - for DOI_fd084c2d1bcf.xlsx` exposed a 27-point
height loss after Default cleanup. The selected rebuilt images differed in
662,040 overlapping pixels, with later content shifted vertically. Original
worksheet XML was 28,220 bytes, the defective output 23,743 bytes.

Empty font cells also explicitly applied default fill and border. The static
style classification excluded them from font-representative selection, although
these attributes masked no visible inherited formatting. Restoring one Calibri
font cell in each of 13 emptied rows eliminated the selected pixel and geometry
differences in a native rebuilt comparison.

The fix selects representatives using effective fill/border appearance for
font-applying styles, while preserving the existing protection policy. The cell
cleanup path now honors that representative decision for these styles too.
The automatically generated fixed output passed the same native comparison.
Its worksheet XML is 25,486 bytes and its archive 18,203 bytes, versus 20,382
original bytes and 17,913 bytes for the defective output. No whole-row retention
was introduced. Core tests cover repeated-cell removal and the existing tail,
protection and fixed-height behavior.

`state-summary-wisconsin-fy2017__ad9c5887-6ede-4454-97bc-4a8ddfe46d04_d844acb6b744.xlsx`
subsequently timed out before either add-in became ready. An isolated first repeat
also timed out. A second exact-input repeat opened Original and Default in
18.30 and 20.96 seconds and found no selected pixel/geometry differences.
Read-only dialog inspection of the explicit owned desktop returned no dialogs.
The startup failure's cause remains unresolved. A normal main-run recheck was
requested through the group restart endpoint; the successful isolated evidence
is retained separately. Launcher metrics now expose `desktop_name` so diagnostic
inspection can verify the actual desktop instead of inferring it from window mode.

## Restart completion bookkeeping

During the same watch, successful manual replacements for Wisconsin,
Pennsylvania and KY-AP-and-IB remained `waiting_cleanup` after their Excel hosts
had closed. Queue throughput stopped. Completion events identified a workbook's
record position; the scheduler marked the first matching entry in its active
history, which was the already-finished predecessor. The live replacement stayed
unfinished and consumed a slot indefinitely.

Completion now selects the unfinished attempt at that position and assigns its
error to that same attempt. A focused test covers both successful and failed
replacement completions without changing the finished predecessor. This fixes
scheduler bookkeeping; it does not explain the intermittent add-in startup
failures that prompted these manual replacements. Durable successful workbook
records were already written and remain reusable after restarting the runner.

The rebuilt scheduler was then exercised through the live restart endpoint twice
on `CO-AP-and-IB_97188f8b40f6.xlsx` in one continuing run. Both replacements
finished `unchanged`, changed attempt IDs and released their active slots. The
remaining queue continued admitting workbooks. The focused scheduler tests and
compact-summary tests also pass.

## Border-masking row headers

`NESCOR DER Failure Scenarios_CSF Subcategories_93e7264d6602.xlsx` exposed a
new vertical line after Default removed trailing rows 148:205. Those rows used
an explicit borderless XF that masked the right border inherited from column F.
The redundant-white-fill shortcut ignored this independent border cancellation.
The flagged range had 2,436 changed black/white pixels at equal geometry.

Restoring only the 58 empty row headers, without their cells, eliminated the
selected native pixel and geometry differences. The fix records whether effective
column definitions contain a visible border and retains explicit borderless row
formats in that case. Shadowed column definitions are resolved by the existing
range map. A focused test covers preservation without cells and removal when the
bordered interval is fully shadowed. All 248 Core tests pass. The actual fixed
output for this small workbook is byte-identical to Original, so the native
pipeline correctly used the unchanged-input proof for its duplicate lane.

## Blank CF inputs and row-local descent metadata

`machine_readable_puf (5)_2daad6b00bf6.xlsx` contains 404 populated/layout rows
but duplicate rules extending down to row 1,048,576. The old run exported at least
964 regions per state before interruption. XML inspection found no changed cell
inputs in rebuilt Original versus rebuilt Default. Two removed untyped blank
rows had `x14ac:dyDescent`; including that attribute in the global CF layout hash
forced every application area into a full capture fallback.

Two distinctions now retain the relevant checks without this global expansion.
Untyped cells without value/formula/inline-string payloads remain blank inputs
when removed, while their visual cell changes remain candidates. Explicit types,
real values and formulas stay conservative. Descent metadata belongs to its row,
so its removal selects the affected rows rather than all CF targets. The streamed
record remains 40 bytes and uses a spare flag bit for explicit nonnumeric types.

On the actual rebuilt files, the read-only selector now reports two logical CF
regions, B403:B403 and B404:B404. A native exact-input repeat selected one merged
capture per state, A385:E512, with no pixel or geometry differences. Its recorded
capture-selection/transport time was 305 ms and image-comparison time 775 ms.
These are phase timings, not total workbook opening time. The targeted suite
passes including explicit typed-blank, string and deleted-value counterexamples.
Earlier v8 results captured a conservative superset and remain reusable.

### Fig 2rev embedded chart export

Office.js enumerated all eight embedded charts but returned `ItemNotFound` for Sheet2 Chart 3 and Chart 4. Activating the worksheet before Office.js export did not resolve the error. Export through the existing PID-validated resident did. Two isolated comparisons exported eight charts and two cell regions in each rebuilt state with no differences. The final test used unchanged Office.js code and restored the previous worksheet/visibility after native export. A focused fake-workbook test covers restoration on success and failure, propagation of export failures, and zero-area evidence. This addresses chart export only; unrelated recalculation timeouts remain under investigation.

### Figure 3 / Figure 4 save warning

Stage telemetry proved that rebuild completed immediately and `workbook.save()` blocked. A disposable visible Office.js reproduction exposed the exact warning: negative or zero values cannot be plotted correctly on logarithmic charts. The original already contains those charts. The narrow acknowledgement handler was tested both on the operator desktop and on the normal private desktop. Each comparison captured three cell regions and three charts per rebuilt state, with no differences. Both states retained acknowledgement receipts. A focused decision test rejects additional message text, other dialogs and multiple actions. Other rebuild/save timeouts have not been attributed to this warning without their own evidence.

### CFU calculation timeouts remain unresolved

Several small CFU workbooks reach `rebuild_waiting_for_calculation` and time out with no detected blocking dialog. A visible isolated reproduction of `2023-06-28 unsealed clay CFU data_9aa57e3b6e57.xlsx` showed Excel's Ready status and an external-link security bar, while Office.js repeatedly reported `Pending` instead of `Done`. The file contains volatile INDIRECT/OFFSET expressions and external links. A separate experiment enabling automatic calculation still timed out after 300 seconds. No security bar was enabled. These observations do not establish the cause, and no Pending response is accepted as a completed rebuild. Explicit timeout results remain unverified; the corpus continues past them. The automatic-calculation experiment is not part of production.

### Supplier diversity content loss

`supplier-diversity-regional-report-2022-extract-4_d48ec4857d1f.xlsx` lost a visible heading and additional string values. Its value elements contain `xml:space="preserve"`; the fast content scanner recognized only literal `<v>` and classified these attributed elements as empty. Both cell classification and the row scanner now accept attributes and XML whitespace after value/inline-string element names, with element-boundary checks. Focused tests cover attributed values, inline strings, whitespace separators and false prefix matches. Core verification passed 250 tests with one ignored. The regenerated file is byte-identical to its source and the native comparison completed without differences, reusing the identical state. The preparation-cache inventory and selective invalidation retain the previous evidence before regenerating exposed artifacts.

### Permit summary full-column conditional formatting

`permitsummaryissuedfebruary2014_65e8c5127739.xlsx` reached the 900-second capture-idle limit during geometry planning, before producing any PNGs. Its duplicate-values rule applies to `B1:B1048576`. Rebuilt Original and Default have identical shared strings and theme, but Excel's saved styles differ (11 versus 9 fonts, 23 versus 17 cell XFs); the existing context guard consequently selects the whole conditional-formatting range. Telemetry recorded more than 5,700 successful diagnosis operations without a completed capture. This is active but excessively costly selection, not a dead worker. No unchanged verdict or weakened conditional-formatting guard was introduced. The failed attempt remains explicitly unverified for a targeted follow-up.

### Rounded uniform-height metadata

`copy of table for 7 nano on BEAS2B 5-9-18_7c9787fb80a0.xlsx` exceeded 250,000 pending height queries. An isolated live diagnostic measured 8,192 uniform visible rows at total height 422,400 points while `format.rowHeight` reported 51.5. The effective mean is 51.5625. The former strict aggregate consistency check consequently subdivided genuinely uniform spans indefinitely. Using the API's uniformity signal and measured aggregate geometry resolved the case. Native verification captured six regions in each rebuilt state without differences; selection/transport took 1,712 ms and image comparison 108 ms. A million-row mock verifies two synchronization batches, while existing mixed/hidden-row tests preserve exact subdivision behavior.

### River-Chla row borders expose empty-column widths

`River-Chla-qPCR dataset_61f40155ea14.xlsx` showed displaced blue borders on Readme. Rows 63–67 carry row-level borders, while column K/L have custom widths and no content cells beyond the cleanup boundary. Merging the column tail changed the visible repeated row-border grid. The fix retains the existing column definitions when visible row borders are present, with one metric propagated through parallel chunks. It does not retain or add whole rows of cells. Core verification passed 251 tests with one ignored. Native verification compared four regions per rebuilt state without differences (selection/transport 1,417 ms, comparison 139 ms). Inventory of 8,623 prepared bundles found 19 exposed variants and zero errors; those variants require backup and regeneration before reuse.

### Proportional geometry planning

The full-column February permit case now completes under partition `visible-geometry-v2`: 2,176 captures per rebuilt state, no differences, 1,059,009 ms for selection/transport and 1,069 ms for image comparisons. The previous binary partition planner exhausted the 900-second no-capture deadline before producing any PNG. Proportional subdivision reduces intermediate native geometry requests while still measuring every resulting child in every state. Tests retain exact union coverage, sparse hidden-row handling, every-state image budgets and explicit indivisible failures. Older measured partitions remain valid evidence because candidate coverage and the pixel comparator are unchanged.

### Exact PNG duplication

The full proportional permit comparison read 4,352 PNGs (856,303,164 encoded bytes) but decoded only three unique encoded images. A separate transport fixture bounded that workbook's CF range to 20,000 rows in both disposable inputs; it is not claimed as full-workbook validation. Native verification compared 42 regions per rebuilt state without differences, with 16,006 ms selection/transport and 201 ms comparison. The pooled uploader retained 92 image paths using eight physical files: 19,948,644 logical bytes versus 2,329,950 physical bytes. Filesystem identities confirmed hard-link sharing; blob names matched exact-byte SHA-256. Focused tests cover authenticated command binding, retries, conflicting content, moving evidence, removing a pool entry, and ordinary-copy fallback. No lossy image changes are involved.

### Fatigue chart-sheet viewport

`1100san11frafat_46e0a2702649.xlsx` originally exported its standalone fatigue chart at different sizes despite unchanged chart parts. An Original/repacked-Original control matched. Enabling `SizeWithWindow` during viewport setup produced matching Original/Default geometry and pixels in an isolated test and again through the Observatory's normal targeted-comparison endpoint. Both reruns measured 724.5 by 442.5 points. The previous view is restored, with a focused fake-workbook test checking viewport adoption and restoration. A separate baseline-repeat shell launch was rejected by automatic tool review; no baseline repeat is claimed. The raw original finding remains retained alongside the successful normalized reruns.

### Native cross-check of CFU Pending state

An isolated probe joined the same owned private desktop and queried the existing Excel process, without opening another workbook. Native Excel reported CalculationState=2 (Pending), Calculation=Manual and Ready=true while Office.js was waiting for calculation completion. Thus the observed Pending state is not solely a stale JavaScript display. The exact cause remains unproven. Microsoft has documented a related volatile-chart Pending-state issue, with a reported fix in a 2025 build; that history is not evidence that this machine has the same defect. No completion rule was relaxed. Reference [OfficeDev issue 3319](https://github.com/OfficeDev/office-js/issues/3319).

### Proportional partition consumers

The completion contract and evidence-retention pass recognize both measured partition versions. Zero-area observations retain their explicit image-free contract, and differing adaptive ranges retain full-resolution context at actual cell-rectangle boundaries. Focused consumer tests cover v1/v2 blank-edge and zero-area validation, v2 adjacent ranges and v2 image-free retention. This closes a version-dispatch gap introduced with the proportional planner; underlying capture coverage and pixel verdicts are unchanged.

### CFU authentication observation

The user encountered an EPA tenant sign-in failure (`AADSTS50020`) after interacting with the CFU workbook's security warning. Read-only observation of that Excel process found a modal `BasicEmbeddedBrowser` with an initially empty native title and a Loading / One moment accessibility subtree. The previous classic-dialog filter returned no dialogs for that same process; the expanded owned-modal filter returned its browser window. This demonstrates a monitoring gap, but does not prove that every CFU Pending-recalculation timeout is caused by the sign-in failure.

Focused classification and private-desktop early-skip tests cover sign-in requests and denied tenant access. The same private-desktop test also reads the accessible error labels through the isolated helper. A separate proposed synthetic password-field test was rejected by automatic tool approval without a detailed reason and was not executed. No authentication was submitted or security warning accepted by these tests.

### Capture-boundary and chart repeat checks

The NWSS data dictionary's Change Log comparison has identical common pixels and comparison cores. Narrowing an empty tail column changes the right capture edge inside overflowing header text. A disposable wider A1:J4 capture retains that text in both variants, with exact common pixels and an entirely white unshared exterior. This is a capture-boundary finding, with the original report preserved. The second cbera model variant likewise has exact common pixels; its sole extra bottom pixel row repeats the preceding row's fill beyond the actual data, after removal of custom-height empty rows 36, 39, 42 and 45.

BrazilSoybeanFigure3 initially differs at 96 chart raster pixels with matching geometry and unchanged worksheet formula/value payloads. A repeat through the production targeted-comparison endpoint has zero cell and chart differences. The raw initial finding is preserved with a review recording nonrepeatable raster variation; its precise cause remains unproven.

The expanded browser-dispatch test exposed an accessibility failure when the reader was launched through ordinary PowerShell startup. Explicit native startup on the named private desktop fixes that failure. The complete private-desktop dispatch, accessible-label read and repeat throttling now pass, alongside the existing desktop-isolation and exact chart-warning tests.

### Escaped worksheet name

`De Facto Water Reuse Bioassay Manuscript Data_b40bf00a15f7.xlsx` failed its height command with `ItemNotFound` for `Fig 6 MDA-KBluc_x0009_`. A disposable Office.js experiment decoding the command's Xstring name completes twelve regions with zero differences. The workbook planner previously decoded XML entities alone. Its worksheet-name boundary now also decodes Xstrings once, preserving escaped literal underscores and joining UTF-16 surrogate pairs. Twenty-three planning tests pass, including escaped-name descriptor lookup, literal escapes and malformed surrogate rejection. The native production-build verification is performed separately from that initial diagnostic experiment.

### Repeated chart names

`Missoula Cs Mass Calculations w ICPMS 041717_488d7cd4c676.xlsx` contains two drawing objects named `Chart 1`, with distinct drawing IDs 2 and 3. The former chart catalogue rejected their duplicate name identity. Inventory and capture now retain collection positions, so both objects are exported independently. Tests cover selection of the second duplicate, name/order mismatch rejection, independent missing-object evidence, and the native one-based fallback.

Native verification with the production worksheet-name decoder and unmodified Office.js commands completes all twelve selected regions, including the actual tab-bearing name, without differences. Native verification of indexed charts completes seven cell regions and both distinct `Chart 1` objects without differences. Their measured left coordinates are 0 and approximately 415.125 points, confirming separate chart exports. Existing completed comparisons with unique chart names remain reusable; failed attempts remain retained for diagnosis.

### Workbook links explain the first CFU Pending case

After the user's observation, a disposable test used the existing owned Excel process's `LinkSources(1)` and `BreakLink(source, 1)` before rebuilding each copy. The original and Default each reported two sources, including EPA SharePoint, and zero remaining sources afterward. Rebuild plus save then completed in 0.264 and 0.605 seconds, and three cell regions plus one chart matched. All seven currently known CFU inputs contain two or three external-link parts. This confirms the cause for the tested case, not automatically for every CFU workbook.

Breaking these links reduces the original's 209 formulas to 144. Thirty-two of the external formulas also contain internal-reference branches. Such a normalized comparison cannot certify preservation of every original formula dependency. The production change therefore records a shorter unverified Pending failure, while retaining the link-breaking comparison as diagnostic evidence. The link-breaking experiment used no extra Excel COM instance and left source/prepared inputs intact.

A second link-breaking test on the Sandy CFU workbook also completes, with three cell regions and two charts matching after normalization. The production Pending guard was separately tested without breaking links: the first CFU input returns `RebuildPendingExternalLinks` after 15.167 seconds, records two input external-link parts and stays unverified. Synthetic state-sequence tests cover active calculation, interruption of Pending by actual calculation, missing external-link evidence, and long Office.js observation gaps during workbook opening.

### First chart source cell supplies number formatting

`Tweedly_-_Millbury_utilities_1__1f81f0ee2c8f.xlsx` changes the Pre-Retrofit chart's primary-axis labels from two decimal places to General formatting. Chart XML is unchanged, including source-linked `0.00` formatting. Its empty numeric series references C7:C18; Default removes those formatted empty cells. Restoring only the original `<c r="C7" s="22"/>` to a disposable Default copy removes all observed cell and chart differences. Restoring only C8 does not. The position-specific counterexample supports a first-source-cell format sentinel, rather than retaining the entire series range. This experiment has not yet changed the shrinker.

A separate audit checked 3,710 retained reports for legacy embedded-chart names that collide case-insensitively within one sheet. It found zero ambiguous catalogues and zero read errors, supporting reuse of prior unique-name chart results after indexed capture was introduced.


The production-path fix now supplies sparse chart format anchors before initial
worksheet cleaning. A native comparison of the newly shrunk Tweedly input has
zero cell-region and chart differences. Its output is 64,342 bytes versus the
previous 64,337 bytes and original 70,114 bytes. Only the first C7 source cell is
restored; C8 remains removed. Evidence: V: validation-temp/experiments/
chart-format-anchor/native-fixed-core-v2/report.json. Parser, empty-row pruning
and streamed ZIP-pipeline tests cover this invariant. Existing corpus results
have not yet been migrated to this new core build.


The prepared-bundle inventory inspected all 8,623 bundles. It found 57 bundles
with 347 missing or changed empty first-source cells; 53 were still prepared,
three had completed without differences and Tweedly had the confirmed difference.
These are invalidation candidates, not 57 proven visual regressions. Identical
Default-to-original aliases require no regeneration. The completed inventory has
no read errors. Original worksheet rows are streamed; rows past the greatest
requested anchor permit conservative early missing-cell detection. An irregular
row order can therefore cause extra invalidation, never acceptance of a changed
anchor. Inventory and migration receipts are retained on V: under validation-temp.

For `Data for Table S6_19a9d184a88f.xlsx`, two chart-sheet plot layouts were
slightly shifted despite equal cell values/formulas. Standard repeat request
`23673d7e-f25c-4467-8f97-f61e0385b7e5` compared all four chart sheets with zero
pixel, geometry or presence differences. The initial raw differences remain;
a separate review links the initial and repeat report hashes and classifies
the observation as nonrepeatable chart layout. No global pixel tolerance was added.


After the previous runner drained normally, all 57 bundles and records were
copied and SHA-256 verified before invalidation. The migration receipt is
`V:/excel-shrink-paper/external/excel-shrink-rust/.tmp/validation-temp/backups/before-chart-format-anchors-20261105-122326/migration.json`.
The full Data.gov run resumed as PID 60916 using binary SHA-256
`d6b97f99c17be451c4958eeeb79b02f618a3f2489f98ac03d40261d9e1522f92`.
The remaining 8,566 prepared artifacts and unaffected completed results were
retained. The long June permit workbook finished its 2,176 candidate regions
without differences before shutdown; its work was not discarded.


### Follow-up: old font cache and empty layout tails

The regular regenerated Tweedly comparison now completes without differences,
using producer `d6b97f99...` and report `targeted-05c3c138-41d9-4e01-abc2-0130483beeff`.

`HGT_STORMS_Checklist_51444494b5ae.xlsx` still used producer `875ad278...`,
which predates the font-sentinel fix. Its original row 1 has a 14-point font
in empty C1, while the old variant removes C1. The current shrinker retains it.
The earlier selective cache screen missed this inherited-border case: the
explicit border is nonzero and matches the row border. A separate audit now
runs the actual current shrinker on legacy bundles with potentially affected
font/fill/border styles and compares their package entry CRC/length signatures.
This is diagnostic work; changed artifacts require backed-up invalidation before
reuse, and audit errors remain unresolved rather than treated as equal.

`PARR transfer data_b3378b41a0c1.xlsx` has values through row 176, explicit cells
through row 178, then an empty row 179 with `ht="15.75" thickTop="1"`. Trimming
that tail changes its measured height to 15 points. Restoring only row 179 to a
disposable current Default yields zero differences in all three selected native
regions. The original differences start below the populated table, where
inherited column borders continue. This establishes the cause, but the scope
of unbounded inherited-column tails still needs review; no tolerance or new
broad row-retention policy was introduced.


### Raster and overflow-context controls

Enlarged glyph crops were reviewed for INL August 2016 oil/BG, SWCNT WetMix,
Ladd 2016 and Final dataset/RUS. The affected charts have isolated antialiased
text-edge changes (1–16 pixels per chart, maximum channel delta 35), matching
cell value/formula payloads and byte-identical chart XML. Separate review records
classify the selected findings under the owner's imperceptible-raster exception.
The diagnostic script now includes RGBA, not just RGB: alpha-only differences
must not be reported as zero changed pixels. Rechecking these four reviewed
cases with alpha included preserves their measured counts. The production
comparator already hashes normalized RGBA and required no change.

For XiaoyuLiu iPCB, Shapefile Metadata and Health Impact Functions, the old
capture boundaries truncate overflow text. Disposable worker copies expanded
only the affected sheet horizontally to A1:Z23, A1:Z11 and A1:AZ52 respectively,
preserving all requested rows. All three native rebuilt comparisons then
complete without pixel/geometry differences after the existing exact-white-
margin rule. The experimental command labels retain the original requested
ranges; review records explicitly retain the actual override and repeat hashes.
Source value/formula payloads match. Production selection was not changed.
The first experimental launches failed before Excel because the copied host
lacked its archive cleanup helper; the complete follow-up copies passed.

Two Mooring findings have identical complete RGBA images, identical dimensions
and exactly matching comparison_core geometry, while outer capture top differs.
These were recorded as selected capture-boundary artifacts, not described as
floating-point noise. Their raw geometry and original verdicts remain available.


### Legacy font-cache audit completed

The audit examined 5,501 prepared bundles from producer `875ad278...`.
It skipped 3,426 exact original aliases and 934 bundles without styles relevant
to the font/fill/border predicate. It ran the current shrinker on all 1,141
remaining inputs with the same Default profile, comparing ZIP entry CRC/length
signatures independently of compression and container metadata. There were
68 output deltas and no execution/read errors. This is a cache compatibility
audit, not a claim that all 68 old variants caused visible regressions.
Nineteen were previously marked unchanged, six had differences and 43 were
still prepared. HGT is among the detected deltas.

After the runner drained and exited, all 68 prepared bundles and records were
backed up and SHA-256 verified before invalidation. The receipt is under
`validation-temp/backups/before-legacy-policy-cache-20261105-125243/migration.json`.
The full run resumed as PID 64928 with the same corrected binary `d6b97f99...`;
these cases must be prepared and compared again. Unaffected records, source
inputs, raw reports and review evidence remain retained. The audit details
remain in `validation-temp/legacy-font-audit-all.json`.


### Observatory cleanup reconciliation

The old Subk directory, Smoot and LTC attempts retain frozen
`cleanup_unconfirmed` snapshots after their telemetry writers exit. Their
recorded Excel PIDs no longer exist, and none occupies a current scheduler slot.
A corrected host tested on port 18771 retires all three stale run IDs, with zero
such active cards and no remaining unconfirmed cleanup in that registry response.
Five focused host tests cover confirmed exit, live/unknown observations, missing
creation time and preservation of running work; frontend syntax checking passes.
The durable timeout errors are not rewritten as successful comparisons.

Automatic approval review rejected replacing the server on 8771 with
`blocked by policy` and no detailed reason. That replacement was not executed.
The old host and runner remain alive; the corrected parallel view at
`http://127.0.0.1:18771/` reads the same study and uses the same control requests.
Only the Observatory host differs; the corpus/Excel processes were not restarted.

The regenerated HGT comparison completes all 513 selected regions without
visual differences. Its row-1 height regression is gone; the remaining row-78
height observation is outside the compared content core. The regenerated food
supplement also has no differences. The four other previously different legacy
supplements now pass the exact RGBA white-exterior diagnostic; their common
pixels match and every unshared exterior pixel is opaque white. Raw verdicts
and images remain. An alpha audit of all 32 existing white/capture-boundary
reviews checked 100 unique image files: every image is opaque, with no read
errors, so the earlier RGB-based boundary proofs are not weakened by hidden
alpha differences. The diagnostic helpers now check RGBA explicitly.

### Unfilled rows masking column borders

Santavy `Table Sup E` (`c1c611c8d8eaeb71f8fe`) exposes another path through
the row-mask issue. Original rows 47–73 use borderless XF 78 with `applyBorder=1`;
several columns inherit visible borders from XF 32. The prepared Default and
the current shrinker both stopped at row 46, revealing a grid in the original
white region. The selected rebuilt comparison has identical capture geometry,
so this is not a capture-size mismatch.

The earlier mask guard covered row processing and final tail pruning. Initial
range selection still recognized only styles with their own fill or border,
discarding the empty masking rows before either guard could run. Range selection
now consults effective column-border masking too. A focused fixture failed
before this change and passes afterward, including the unbordered negative
control and both split settings. All 255 active core tests pass (one ignored).
An actual shrink of the workbook now retains its 73 row headers; native Excel
counterchecking and selective cache migration follow separately.

The private-desktop native countercheck subsequently completed all 212 selected
regions with no pixel, geometry, chart-rendering or chart-presence differences.
Its report is retained under
`validation-temp/santavy-current-control/native-2/report.json`. A separate review
links the raw failing report to commit `14b5029` and this repeat; the original
finding is unchanged.

The prepared-cache inventory examined all 8,623 bundles without errors. It
checks explicit borderless row headers against visible column-border styles
and the retained Default row extent. Two bundles require regeneration: Santavy
and FractureSustainabilityGeothermal Chemical data summary
(`72415f53ff6a8589442b`). Both bundles and records were copied to a SHA-256-verified
backup before invalidation. Other prepared artifacts and completed results
remain reusable. The ongoing large Figure 4 fiber workbook was interrupted
explicitly for the build switch and remains unverified/retryable.

The full run resumes with the border fix, four groups and a 1,024 MiB reserve.
Its new control host uses port 18773; earlier control hosts remain running rather
than being replaced. They read the same study, but the new host is the primary
view for this resumed build.
