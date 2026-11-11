# Ordinary configuration starter verification

On 2026-11-11 the ordinary `tools/start-deltasieve.ps1` starter was exercised
with its new `-StudyConfig` argument and an existing release executable.
Executable SHA-256 was
`58f138b706a16831b50eb7de64cf3c6b4b8bc88a7db9f9444b96665efd5b1854`.
This is a prior frozen build. The starter change supplies an already supported
CLI argument and does not require rebuilding that executable.

The private HTTP fixture supplied a configuration filename containing spaces,
six size partitions with partition four selected, two conversion workers,
three shrink processes, one thread per shrink process and two comparison
groups. Check Performance was disabled and equal-image retention enabled.
The session endpoint returned all these settings. Relative archive and output
paths resolved against the configuration file's directory, including Windows'
extended canonical-path prefix.

The server returned HTTP 200 through `deltasieve.localhost`,
`configure.deltasieve.localhost` and `localhost` on the selected explicit port.
An unlisted subdomain returned HTTP 403. A second starter using the occupied
port exited with a nonzero status. The original listener still returned 200.

The state endpoint reported `active=false`, an idle progress state and no
preparation or corpus job. The configured study output directory was absent.
The fixture made only HTTP GET requests and terminated its own starter process
tree afterward. The two existing native comparison workers and their capture
budget watcher remained live.

Private fixture and full receipt are retained under
`G:\deltasieve-work\ordinary-start-prefill-20261111`. They include local paths
and are excluded from public source. This check verifies configuration handoff,
HTTP service identity and port-conflict behavior. Browser rendering, a native
study start, relocation and receipt-based native resume require their separate
acceptance checks.

## Browser finding and correction

The subsequent browser check exposed an application-routing gap. The DeltaSieve
shell received study defaults but hid its SheetShrink-only study modes. The
SheetShrink application's newer cleanup Observatory also intercepted
`observatory --study-config` and rejected the option. Successful HTTP prefill
alone had not established a usable corpus form.

The corrected starter requires `-Application sheetshrink` for archive-study
prefill. The SheetShrink application now routes this explicit command to its
shared corpus configurator. The configurator selects the new-study form when
that application has a supplied nonempty study output root. Unconfigured cleanup
continues through the detailed cleanup Observatory.

A guarded release build and two application-routing tests completed. Two
PowerShell starter tests, five study-configuration JavaScript tests and six
runtime-package tests passed. The tested SheetShrink executable SHA-256 is
`b29a46ce04ab55d4e3b52cedc395d4f3c6cd1b5de53cafc32e88c8713dc12ea5`.
The browser displayed the selected new-study workflow, partition 4 of 6,
two conversion workers, three shrink processes, one thread per process and
two visual groups. Check Performance remained unchecked, and the Start corpus
study button was available. No Start action was performed. Native execution
and resume remain outside this check.

## Completed-study resume

The ordinary SheetShrink starter subsequently opened a separate configuration
host. Its authenticated same-origin resume endpoint accepted the earlier
one-workbook study's saved receipt. The exact recorded runner completed in
11.594 seconds. The launch receipt, request, configuration and result-record
SHA-256 values remained unchanged, and the single capture-attempt directory
set was unchanged. The test's owned process tree was then stopped.

The outer guard allowed 180 seconds and 4 GiB aggregate job commit, with an
8 GiB physical-RAM and commit reserve. Peak job commit was 160,387,072 bytes.
Minimum free RAM and commit headroom were 67,968,643,072 and 50,760,081,408 bytes.
The path-free receipt is in
[`ordinary-web-resume-20261111.json`](ordinary-web-resume-20261111.json).
The full private driver and receipt remain beside the earlier prefill fixture.
This verifies completed-study evidence reuse through the ordinary web resume
adapter. Interrupted-phase recovery and portable relocation keep their own
acceptance requirements.
