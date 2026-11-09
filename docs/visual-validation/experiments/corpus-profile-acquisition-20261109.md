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
restart and are rechecked on reuse. Study creation from materialized profile
members, partition integration and the multi-machine rehearsal remain required.
