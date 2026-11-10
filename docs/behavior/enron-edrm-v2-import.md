# Enron EDRM v2 acquisition and imported extraction

## Confirmed source

The user confirmed the Internet Archive item
`edrm.enron.email.data.set.v2.xml` on 2026-11-09. Its retained metadata identifies
the collection as EDRM Enron Email Data Set v2 XML and lists its creator as Enron.
The frozen source profile contains 159 ZIP URLs totaling 79,087,225,630 bytes.
It preserves multipart custodian ZIPs, including the Kaminski and Kean parts.
Each ZIP retains the provider's byte length, SHA-1 and MD5. The profile does not
invent a provider SHA-256. Local archive verification should additionally record
a SHA-256 when those bytes are available. [Internet Archive collection](https://archive.org/details/edrm.enron.email.data.set.v2.xml)

`profiles/corpora/enron-edrm-v2.json` uses source-profile version 2. Its ZIP list
describes the acquisition inputs. Extracted workbook coverage is unknown until a
workbook inventory is available. The common catalog therefore exposes an unknown
workbook count and explicitly unavailable acquisition. The web selectors display
the profile as awaiting its extraction package. The start API rejects it before
filesystem or download work. Single-ZIP Data.gov and Destatis profiles remain
available under their existing workbook-manifest contract.

`tools/build_enron_source_profile.py` regenerates the source list from a retained
Archive.org metadata document and an explicit retrieval timestamp. The source
profile records that document's SHA-256. The retained response is presently in
the private `validation-temp/enron-v2-profile` folder. Only metadata was fetched;
the 159 ZIP archives have not been downloaded by this preparation.

## Required integration when the user supplies the extraction package

The user is finishing extraction on another machine and will supply the scripts,
outputs and available logs together. The following contract guides that adapter.
Import and full reconstruction are still implementation work.

1. Freeze the extraction recipe, script hashes, relevant tool versions, arguments
   and selection rules. Include recursion, supported container formats, attachment
   decoding, duplicate handling and failures that affect the output inventory.
2. Bind the recipe to the exact source-profile revision and all selected ZIP
   identities. Keep locally verified input archives distinguishable from source
   identities declared by an external extraction receipt.
3. Build an output inventory with stable relative paths, formats, byte lengths
   and SHA-256 hashes. Retain original XLS, XLSX and other supported workbook
   formats separately from converted baselines. Preserve occurrence identity
   when identical bytes were found in multiple messages or containers.
4. Retain container and attachment origins where the extraction provides them.
   Record missing lineage explicitly. File-name similarity alone cannot prove
   which upstream attachment produced a workbook.
5. Verify imported files against the output inventory before reuse. Record
   `externally_generated` provenance and the actual extraction environment.
   Local verification of file hashes does not prove that the entire recursive
   source extraction was reproduced. A supplied inventory can define the selected
   evaluation set while whole-source extraction coverage remains unverified.
6. Provide reconstruction through the same frozen recipe and source collection.
   Once a compatible imported extraction is verified, reuse its inventory and
   files without repeating the multi-day download/extraction work. Keep missing,
   changed, failed and verified artifacts distinct.

## Receiving-side acceptance on 2026-11-09

The supplied handover contains 34,065 distinct candidate payloads totaling
8,469,579,995 bytes, with 156,753 direct candidate locations. The full receiving
verification reread every candidate and matched its SHA-256 and size. It also
matched the frozen pipeline code hashes and snapshot candidate counts, with no
pending or working inspection entries. All 159 declared archive URLs, sizes,
SHA-1 and MD5 values match the versioned source profile. The source archives
themselves were not included or locally reread.

The five supplied `test_prepare_enron.py` tests passed locally after installing
their missing dependencies in an isolated environment. This bounded acceptance
does not reproduce the full extraction. Three tests initially failed dependency
preflight because `olefile` was unavailable. The rerun used the declared versions
of olefile, oletools, psutil, pywin32 and pypdf. Transitive dependencies were
resolved locally and are not a byte-identical reproduction of the source runtime.

The inventory declares 5,297 `legacy_excel` and 28,768 `ole` candidates. Review
states and unresolved extraction cases remain part of the selection provenance.
Receiving-side hash verification establishes transfer integrity. Openability,
conversion acceptance and complete upstream extraction coverage require their
own evidence. Import into the ordinary runner and reconstruction integration
remain open.

## Import command

`deltasieve corpus-import-enron --handover <directory> --output <new-directory>`
verifies the declared 159 archive identities against the built-in source profile,
candidate counts and lengths, frozen code receipts and every candidate SHA-256.
It streams the original bytes into a deterministic, uncompressed candidate ZIP.
Members are sorted by SHA-256 and use `candidates/<sha256>.xls` so the existing
archive runner can submit each candidate to its legacy conversion path. This
extension is an input-routing convention. It does not certify the underlying
candidate as an Excel workbook. Native conversion must still accept the file.

The import preserves the manifests, occurrence records, review cases, scripts,
receipts and external ledger in a separate `handover` directory. `import.json`
is published atomically after both the candidate archive and copied metadata
are verified. It explicitly declares external generation, distinct-payload
selection and unverified openability. The original handover remains unchanged.
An interrupted import retains its partial output for inspection. A retry chooses
a new output directory, and existing imports are never overwritten.

The resulting `candidates.zip` is accepted by the existing custom-archive study
entry point. The built-in Enron download/profile workflow remains disabled until
recipe reconstruction and its ordinary web-profile integration are implemented.

## Evaluation states

For an XLS original, the Excel-generated XLSX is the saved baseline. Apply
SheetShrink and native Check Performance as independent producers to that same
baseline, retaining producer and Excel environment provenance. The user removed
the extra ordinary-save branch for these already converted XLS inputs. Capture
and rebuild policy remain the existing versioned validation contract.

The previous small Enron corpus was intentionally stopped by the user. Keep its
prepared files and raw results as historical evidence. It is not the complete
v2 extraction and is not automatically resumed while this replacement is prepared.

## Microsoft Graph postprocessing

Some extracted candidates are Microsoft Graph OLE objects with CLSID
`00020803-0000-0000-C000-000000000046`. Excel rejects these as chart gallery files.
`tools/recover_graph_workbooks.py` identifies this class from the OLE root entry
and runs a separate, resumable data-recovery stage using desktop PowerPoint.
The original candidate inventory and occurrence ledger remain the inputs to
this stage. Its outputs have their own `microsoft-graph-derived-data` inventory.

Run download, recursive extraction and Graph recovery through the same wrapper
using the preserved extraction modules from the handover artifact:

```powershell
python tools/recover_graph_workbooks.py `
  --pipeline <handover>/scripts/pipeline `
  --source-url https://archive.org/details/edrm.enron.email.data.set.v2.xml `
  --seven-zip <path-to-7z.exe> `
  --extraction-workspace <new-or-resumable-extraction-workspace> `
  --output <separate-graph-output>
```

For an existing extraction workspace, omit `--pipeline`, `--source-url` and
`--seven-zip`. For an imported candidate ZIP, use `--candidates <candidates.zip>`
instead of `--extraction-workspace`. Both routes use the same recovery worker.
The frozen handover scripts do not need to be modified. The ordinary web-profile
download entry remains disabled as described above. This wrapper is the explicit
download/extraction/postprocessing entry point.

The worker generates a blank PowerPoint carrier and embeds exactly one original
Graph payload. Its original presentation is unnecessary. A warm PowerPoint
process converts successive objects. A compiled C# UI Automation monitor handles
only the English chart-conversion question in that process. The worker extracts
the single generated XLSX, validates its ZIP CRC and workbook part, and records
the source hash, output hash, Office version, recipe hashes and duration.

These XLSX files contain the recovered chart data. The modern chart resides in
the temporary PowerPoint carrier. The recovery does not establish visual
equivalence, retain an original workbook layout, or automatically add these
derived files to a previously frozen workbook evaluation cohort.

Windows desktop PowerPoint, Python, olefile, pywin32 and psutil are required.
Use an otherwise unused interactive PowerPoint session. Opening PowerPoint for
other work during recovery would break this isolation. The worker activates its
carrier window and sends window-scoped double-click messages. Macros are disabled
for its COM application. Other security or repair prompts remain unanswered.
RAM and system commit reserves default to 8 GiB each, with an 8 GiB output-drive
reserve. A deadline stops an unresponsive worker using its recorded PID and
creation time. It refuses to attach to a pre-existing PowerPoint process.

Verified successful outputs are reused after a restart. Failed receipts require
`--retry-failed` for another attempt, with the previous receipt retained. Put
`stop.request` in the output directory to stop the controller. Remove it before
resuming. `--limit` bounds new attempts for a pilot. `status.json` reports the
controller state and `derived-workbooks.json` inventories the recovered data and
failed objects. An unbounded run restarts its owned worker after a durable
blocked-object receipt and continues with the next object. Three restarts without
progress stop the run. Memory, storage, startup and integrity failures stop it
without this automatic restart.

On 2026-11-10, five different real Graph objects converted in one warm PowerPoint
16.0 process. A repeat reused all five verified outputs without starting
PowerPoint. The first object was independently traced to slide 29 of its parent
presentation, “Evolution of the generation mix”. Its recovered worksheet contains
Hidraulica 0.40, Nuclear 0.19 and Térmica 0.41, matching the earlier manual
PowerPoint conversion. A larger pilot recovered 14 distinct objects. An initial
UI Automation race raised `ElementNotAvailableException` when a dialog closed
during enumeration. The monitor now re-enumerates after that specific exception.
These bounded tests do not reproduce the multi-day source download/extraction.

The larger pilot subsequently recovered 50 distinct objects. Graph payload
`02b6604f968797eebc9fe588cb111c791fe7ca69aa95580cebb410e2b6da174f`
produced PowerPoint's explicit message that the chart cannot be converted to the
current Office chart format. The monitor acknowledges only that specific failure
message and the object receives a failed receipt. The following objects continued
successfully in the same process. Short `RPC_E_CALL_REJECTED` and server-busy
responses during native conversion are retried within a ten-second deadline.
