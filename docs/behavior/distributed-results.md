# Distributed result collections

`deltasieve merge` assembles finished standalone DeltaSieve reports into a new
collection. It verifies and copies the portable packages used by static export.
Source inputs remain read-only.

```powershell
deltasieve.exe merge 'V:\results\partition-1' 'V:\results\partition-2' `
  --output 'V:\results\combined'
deltasieve.exe export 'V:\results\combined' 'V:\publication\combined'
```

The destination must be new and separate from every source. An interrupted or
failed publication keeps `.incomplete` and copied files for inspection. Readers
reject that directory as unfinished. Retry into a new destination after resolving
the failure. Success removes the marker after the viewer and inventory are written.

## Identity and provenance

Each input location receives a namespace derived from its canonical location and
original run-metadata checksum. Repeated references to that location/snapshot are
included once. Separate directories with identical metadata are independently
verified and retain separate observations. Metadata equality cannot bypass
package integrity checks.

Original `run.json` bytes are preserved as `source-run.json` under each namespace,
with SHA-256. The portable snapshot, configuration, selection, validation
signature, preparation provenance and original times remain inspectable. Pair
identifiers are namespaced. Technical names and scientific reports remain intact.
Preparation measurements retain their producing-machine attribution.

Counts describe observations and do not establish unique corpus coverage. No
conflicting result is chosen or overwritten. Potential conflicts are flagged
where complete package fingerprints, technical names, available source identities,
the validation signature and a nonempty declared environment match, but status
or difference-count summaries disagree. Matching summaries do not prove identical
findings. Missing/partial identities and different environments stay independent.

Source replays retain their own clocks and source pages. Static re-export keeps
those pages, metadata and replay assets. A publication inventory covers collection
and source metadata. Workbook content and distribution rights still require review.

## Release gates

This command initially accepts standalone version-one reports. The archive
corpus-study adapter and collection-aware result reuse remain separate work.
Supply original source reports for reuse meanwhile. The distributed release also
requires automatic hardware/Excel environment capture, unified profiles, portable
Check Performance acceptance and a multi-machine start/resume/export/merge
rehearsal. These subset checks cannot establish those release gates by themselves.

## Acceptance on 2026-11-08

Six synthetic merge tests passed, as did the three shared site tests, portable
frontend test and workspace check. Two completed native SheetShrink web reports
were then collected through the actual CLI and exported again. Both original
run metadata files retained their SHA-256. Browser inspection confirmed the two
observations, source report navigation and the source replay's terminal
`completed` checkpoint. The owned loopback server and tab were closed.
Private receipts remain in
`G:\deltasieve-work\distributed-merge-acceptance-20261108`.

This was a same-machine report-format acceptance. It did not substitute for the
distributed multi-machine rehearsal or establish pixel-detection accuracy.

### Automatic standalone environment snapshot

New standalone run headers record an environment object containing the executing
binary's SHA-256 and a hardware/installed-Excel snapshot. The collector is shared
with archive runs and reads CIM/registry metadata without starting Excel. Missing
fields retain collection warnings. Standalone snapshots omit hostname and label,
using the hashed machine identity for attribution. Historical reports remain
unchanged. This metadata is observational and does not replace the declared
renderer environment key used for result reuse.

The focused failed-source/replay test passed with assertions for the executable
checksum, machine identity, Excel metadata object and absent hostname/label.
Collection and storage succeeded even for a run whose source preparation failed.
This completes metadata publication for that standalone entry point. Archive
collection adaptation and the multi-machine rehearsal remain open.

## Selected observation exports

A staged report can reuse terminal observations from a standalone run by selecting recorded pair IDs.

```powershell
deltasieve.exe export 'V:\results\full-run' 'V:\publication\stage-09' `
  --pair-id pair-one --pair-id pair-two
```

Unknown and repeated IDs are rejected before creating the destination. Selected
packages retain their original verdicts and integrity manifests. Unselected
packages are omitted. The source run remains unchanged. Its exact metadata bytes
are retained in `selection-source-run.json`, with SHA-256 and the selected IDs in
`observation_selection`. Overview counts describe the selected observations.
The original timing still describes the entire source run. Its status and full summary
remains in the selection provenance. The broader replay is unavailable for this
subset. Re-export and subsequent merge retain and verify the original metadata.
Select from the original standalone report before merging. An active source is
allowed only when every selected comparison is terminal. Selection status and
counts describe that snapshot, while the full run keeps working. Publication review
also includes the retained full source metadata.

Atlas labels may explicitly set `variant_group` alongside `corpus` and `variant`
when independently retained runs belong under one presentation selector. The
shared key and display name determine this presentation group. Observations keep
their separate IDs, run IDs, native evidence and validation signatures. Grouping
does not establish methodological equivalence. Use separate keys and names for
normalized baselines and distinct tool profiles. Without this explicit field,
variant identity retains the original per-run behavior.

## Frozen Atlas cohorts

`deltasieve atlas REPORT --cohort cohort.json` retains a presentation selection
as `atlas-cohort.json`. Its schema is `{"schema":1,"workbooks":[...]}`. Each
workbook entry needs `corpus`, `name`, `original_sha256` and a positive `rank`.
Ranks are unique within each corpus. Duplicate identities and malformed hashes
are rejected. The Atlas orders each corpus by these ranks before other entries.

An expected workbook stays visible even with no recorded observations. It has
an unknown sheet inventory and no invented comparison verdict. Actual matching
original identities supply the recorded sheets and observations when available.
Different original hashes remain separate. Re-export carries the cohort file.
This presentation selection does not alter native results or establish coverage.

Standalone export snapshots also retain a workbook inventory when the original
is available locally. This records its SHA-256 and ordered sheet names/states,
without worksheet-cell inspection. The Atlas prefers the inventory attached to
the native report, then this snapshot inventory, and finally local recovery.
A failed comparison keeps its failure status. Re-export can therefore preserve
its original identity and known sheets after the source files are unavailable.
Unavailable inventory remains unknown.

`deltasieve publication-review EXPORTED_REPORT` prints a bounded inventory of
publication concerns without changing the report. For a merged export, review
covers every copied JSON and gzip JSON file, including nested comparison reports,
source snapshots, Atlas data and replay channels. It also parses the exact
`report-data` JSON block in generated report HTML, including comparison pages.
HTML without that block is skipped. Malformed embedded report JSON fails the
review. The output counts JSON files and embedded reports separately. HTML
input has the same 128 MiB read limit. This metadata check does not inspect
arbitrary HTML text, script code, workbook contents or image pixels.
It excludes prior review
inventories so findings are not recursively counted. Compressed and expanded
JSON are each limited to 128 MiB. Replay verification retains its stricter
independent transport limits. Findings contain file names and JSON pointers,
with a complete count and at most 1,000 retained entries. This inventory neither
redacts evidence nor certifies publication rights or anonymity.

### Public Atlas presentation

`deltasieve public-atlas EXPORTED_REPORT NEW_DIRECTORY` derives an Atlas-only
website from an existing static export. It verifies each referenced comparison
package and requires Atlas detail data to match that package. It copies images
byte-for-byte and retains the scientific comparison, formula and dependency
payloads. Invalid evidence fails the export, leaving an `.incomplete` marker.
Existing destination directories are rejected.

The presentation excludes native run configurations, identities, executable
programs and replay archives. Local transition paths, process identifiers and
diagnostic error text are omitted from the derived JSON. Omission records contain
JSON pointers and original-value SHA-256 hashes. Source report and catalog hashes
remain in `public-provenance.json`. Environment keys become hashes so distinct
environments still have distinct comparison-context signatures. The source
export remains unchanged. Derived report pages also identify their public
presentation provenance.

This directory is a presentation artifact and cannot resume or merge as a native
run. Its viewer supports the retained Atlas interactions and finding pages.
Replay remains available in the original export. Publication review still runs
on the derived directory. Use `deltasieve serve DIRECTORY` to open it locally
and `deltasieve publication-review DIRECTORY` to inspect its publication metadata.
Both accept the Atlas directory containing `index.html` and
`atlas-data/catalog.json.gz`. Workbook names, formulas, cell contents, screenshots
and distribution rights require review before public deployment. These scientific
payloads are deliberately retained even when their contents resemble local paths.

### Explicit native reference observations

`deltasieve import-reference REFERENCE_DIRECTORY NEW_REPORT --workbook NAME
--corpus CORPUS --variant TOOL --scope-note EXPLANATION` adapts a completed
`explicit-reference-plan-v1` receipt to the ordinary Atlas/report format.
The importer verifies the retained rebuilt workbooks and every image against
the native receipt before creating a destination. It preserves the exact source
receipt text and its SHA-256, inventories the retained workbook and associates
that inventory with the receipt's original-input identity. It copies scientific
geometry and difference observations without rerunning Excel or candidate
selection. Original image encoding is retained for matching cross-tool contexts.

Imported references always use a separate corpus view with the suffix
`(explicit reference controls)`. Their selection metadata sets `recall_evidence`
to false. The required scope note must disclose posthoc selection when applicable.
These observations can illustrate a measured difference and compare identical
reference contexts. They do not complete an ordinary targeted comparison, establish
whole-sheet coverage or supply a blinded recall estimate. The original source
directory remains unchanged. Ordinary merge, export and public-Atlas presentation
are available for the derived report.

The Atlas difference mask and its automatic magnifier use only pixels inside
both native image extents. Canvas padding remains visible as missing image area
and does not create a highlighted pixel or choose the magnifier location.
Recorded geometry, presence and rendering verdicts remain unchanged. Grouping
of identical captures still requires identical full displayed pixel arrays.

Within a comparison package, byte-identical equal PNGs reuse the result of the
first compression attempt. This includes the original encoding when the attempt
did not reduce the file size. Captures required for differing or unverified
observations retain their original encoding.

## Hosting the staged comparison Atlas

GitHub Pages limits a published site to 1 GB. Measure the entire assembled
site before deployment, including other stages already hosted there. The
size reported by `public-atlas` describes that export only.

In the partial stage-nine preview assembled on 2026-11-10, the manifests
referenced 2,490,155,707 encoded image bytes for ordinary Data.gov comparisons
alone. Destatis and Enron added 552,194,161 and 146,546,638 bytes, respectively,
before normalized comparisons, controls, thumbnails, and viewer assets.
Splitting by corpus therefore would still leave Data.gov over budget.

Keep the complete self-contained projection as the reference publication
artifact. If it exceeds the limit, a Pages deployment needs independently
hosted static evidence assets or a link to the complete Atlas on a host
that accommodates it. For a split deployment, acceptance requires opening
deep links over HTTP and exercising every image consumer, including overlays
and zoom views that read pixels through a canvas. Remote images must permit
that access from the final site origin. A release download can preserve the
complete artifact for offline use but does not establish that the interactive
page can load individual captures. Verify the deployed site before adding
its URL to the manuscript.

[GitHub Pages limits](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits)

The public projection validates copied capture files against the source
package manifest. This checks each published capture's length and SHA-256
without an additional source-image hashing pass. An integrity failure
leaves the public destination marked incomplete.
