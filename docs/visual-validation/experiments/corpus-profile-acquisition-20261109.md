# Corpus profile acquisition acceptance

## Scope

The embedded Data.gov and Destatis manifests drive the CLI catalog, local web
catalog and acquisition utility. This experiment checks source identity,
non-overwriting publication, background-job ownership and bounded status. It
does not establish profile-driven study creation or distributed-run acceptance.

## Focused checks

- Seven `validation_corpora::` unit tests cover manifest identities, read-only
  inspection, missing-member restoration, complete offline reuse, conflicting
  originals, archive and member hash rejection, partial retry, exclusive
  acquisition, create-only publication and phase/member progress.
- The exact test
  `validation_room::corpus_job::tests::read_only_job_finishes_after_request_and_keeps_polling_compact`
  covers active-operation rejection, immediate acceptance, background completion,
  compact missing counts and absence of inspection writes.
- Three browser-module tests cover request construction, visible status, custom
  directory preservation, profile changes, job submission and terminal rendering.
  The two existing study-configuration tests also pass.
- The existing host publication test
  `capture_host::tests::evidence_cannot_overwrite_a_previously_published_result`
  passed when the acquisition CLI adopted that helper in the preceding step.

Tests and builds used the validation temp/build directories on V:. The generated
ZIP fixtures contain tiny byte payloads. They test acquisition integrity rather
than Excel readability or visual equality.

## Local HTTP acceptance

The debug DeltaSieve executable started a separate configurator on port 18831,
without opening a browser or starting validation. A real HTTP session retrieved
both catalog profiles and submitted a token-authenticated Data.gov inspection of
an absent originals directory. The accepted job subsequently completed with 9,117
missing originals. Its serialized job snapshot was 570 bytes. The HTML included
the acquisition form and the new module returned HTTP 200. Inspection created no
originals directory. The owned test host PID 35688 was then stopped.

The private receipt and host logs are in
`validation-temp/corpus-web-acceptance-20261109/acceptance.json` and its adjacent
logs. This checks HTTP delivery and background behavior. Full visual browser
acceptance and a real large-archive acquisition remain open.

## Interrupted broad test selection

An initial `corpus_` test filter also selected the pre-existing
`corpus_regression_smoke_suite`. That suite began downloading an 89,367,137-byte
Destatis workbook and running the large cleanup fixture. The identified test
process was stopped. This interrupted suite is not recorded as passing. The
downloaded workbook and its two newly created timing logs were moved from C: to
the experiment's `smoke-interruption` directory on V:. Subsequent acceptance used
the exact web test name and the `validation_corpora::` module filter. The
production Enron process was not stopped or reconfigured for these checks.

## Remaining integration

The utility does not automatically start a corpus study. Acquisition status is
retained for the configurator server lifetime, while verified originals survive
restart and are rechecked on reuse.

## Profile-backed study integration

The study form now accepts a profile with originals and cache directories. It
waits for the exact background acquisition job to finish successfully and release
admission before creating the study. Creation rechecks the sources and freezes a
directory-backed profile request. An empty profile retains the ZIP workflow.

Two small tests prove catalog fingerprint and size-plan equivalence, including
non-workbook ZIP members and reordered manifest entries, plus private-copy hash
checking before processing. Eight study-configuration tests, eleven existing
selection tests and the original ZIP extraction test pass. Seven profile tests
also pass. Seven JavaScript tests cover configuration, source acquisition and
the handoff gate. A new configuration test initially compared extended Windows
paths with unprefixed paths; it now compares the canonical parent consistently.

An additional offline acceptance used the already retained, pinned Destatis ZIP.
The debug executable restored and verified all 1,182 originals, totaling
2,364,066,555 workbook bytes. The observed debug preparation wall time was 286.22
seconds; this is an integrity check, not a throughput benchmark. No source ZIP
was downloaded for this acceptance. Selection-only invocations then generated
plans from the directory and ZIP with the same corpus ID and size partition 1/4.
Their complete source descriptors, fingerprint and selection matched. Both
selected 295 of 1,182 members. These commands did not start Excel comparisons.

The private preparation and selection receipts are under
`validation-temp/profile-study-20261109`, including `prepare.json` and
`selection-acceptance.json`. Full visual-browser acceptance, a native profile
study through its web handoff and the multi-machine rehearsal remain open.

## Full Data.gov acquisition and legacy Unicode names

The optimized profile acquisition first restored 6,813 originals, then rejected
ZIP member index 6814. Its registered name is
`Ramhøj et al._ PFHxS and Thyroid_means and SD_21.02.2019_d9619a6370e9.xlsx`.
The legacy header contains byte `0x9b`, which the ZIP library interprets through
CP437. The same entry carries a version-one Info-ZIP Unicode Path extra field
with a matching raw-name CRC and the registered UTF-8 name. The whole source
archive had already passed its pinned size and SHA-256 check.

The acquisition path now accepts that CRC-bound Unicode identity when the
ordinary decoded name differs. Member indices, registered output paths, byte
counts, content hashes and non-overwriting publication retain their existing
contracts. Unknown versions or stale name checksums supply no Unicode match.
Malformed and ambiguous valid fields are explicit errors. The interpretation
follows [PKWARE APPNOTE section 4.6.9](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).

Eight focused acquisition tests passed, including an actual ZIP fixture with
different legacy and Unicode names, strict CRC/UTF-8 handling, partial reuse,
conflict preservation and exclusive publication. The fixture initially required
a correction because the ZIP writer reserves this extra-field ID behind an
optional feature. It now constructs the reserved metadata by patching the two
documented test-field headers, with no production dependency change.

The ignored test `inspect_pinned_datagov_archive_member_names` was then run with
the explicit private archive path. It verified the complete archive SHA-256 and
all 9,117 registered member names, sizes and CRCs. Exactly one name required the
Unicode alias. This debug test took 540.74 seconds, primarily including the
complete archive hash. Its elapsed time is an integrity-test observation.

The optimized ordinary `corpus-prepare` command resumed the existing originals,
added 2,304 files and reported 9,117 verified, zero missing and zero changed
originals. The initial 6,813 successful publications remained reusable. Its final
receipt and logs are retained under
`G:\SheetShrink-Night-20261109\acquisition-fix`. The archive used is the same pinned
Data.gov ZIP distributed with the offline package. This acceptance proves full
source acquisition and hash verification. Native comparison outcomes are tracked
independently by the sequential night queue.

## Corrected portable runtime acceptance

The separately packaged `runtime-1f72e78b` contains 114 verified files. Its
DeltaSieve executable SHA-256 is
`6cd1fa9eb0d5afb72b4bd6a8b3f2183607dcfdf535038e3c5b72a915874e80b7`.
An ordinary native chart control completed in 11.8597733 seconds and retained
the expected deliberate difference. Both image hashes matched the previously
retained control. The acceptance receipt SHA-256 is
`201abc1ec15b183c3a1fa431593ccca69e02b23553277f0a02e789d23e98856f`.

A private runtime-only update ZIP was checked for ZIP CRC integrity and every
packaged file's size and SHA-256. It contains no corpus or personal starters.
Its SHA-256 is
`dddc4b6b05911da2be0756f364d9b662544518512172466da4b3412298d09682`.
The existing night comparisons continue using their pinned `0ad30f53` runtime.
This acceptance does not prove existing-study migration compatibility.
