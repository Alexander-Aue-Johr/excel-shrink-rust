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

## Evaluation states

For an XLS original, the Excel-generated XLSX is the saved baseline. Apply
SheetShrink and native Check Performance as independent producers to that same
baseline, retaining producer and Excel environment provenance. The user removed
the extra ordinary-save branch for these already converted XLS inputs. Capture
and rebuild policy remain the existing versioned validation contract.

The previous small Enron corpus was intentionally stopped by the user. Keep its
prepared files and raw results as historical evidence. It is not the complete
v2 extraction and is not automatically resumed while this replacement is prepared.
