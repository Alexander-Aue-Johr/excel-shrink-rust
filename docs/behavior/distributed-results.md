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
