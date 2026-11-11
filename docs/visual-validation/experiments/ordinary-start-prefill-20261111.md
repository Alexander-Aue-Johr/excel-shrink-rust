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
