# Copied-runtime startup acceptance

The release candidate was packaged on G: and copied to a directory containing
spaces on V:. All 132 manifest entries were verified before and after testing.
The ordinary packaged starter used its bundled executable and web/Office assets,
with its working directory set to the package. Its default data root was the
external sibling `deltasieve-data` directory. No repository working directory
or explicit executable override was used.

Both DeltaSieve and SheetShrink configuration hosts became ready in approximately
one second. SheetShrink retained the requested study partition 4/6. Its source
and output paths remained those supplied by the configuration file. This test
does not redefine them relative to the copied executable. The configuration
host's JavaScript matched the packaged asset bytes. Both owned test hosts were
stopped and package contents remained unchanged.

The first package deliberately retained the previously supplied DeltaSieve
binary, SHA-256
`58f138b706a16831b50eb7de64cf3c6b4b8bc88a7db9f9444b96665efd5b1854`.
Its embedded JavaScript differed from the fresh packaged file. This passed
manifest integrity but failed the web-asset compatibility check. The failure
is retained separately. Rebuilding the CLI binaries from the current source
and publishing a new package corrected the mismatch.

Successful DeltaSieve binary SHA-256 is
`c6a214e19b7839bda4775f82baa153f9ed3db261caa2df01ec0cb27fa060856c`.
SheetShrink SHA-256 is
`b29a46ce04ab55d4e3b52cedc395d4f3c6cd1b5de53cafc32e88c8713dc12ea5`.
The reusable `tools/check-runtime-web.py` passed on the corrected package and
rejected the earlier mixed package with the expected embedded-JavaScript error.
It checks both JavaScript and CSS and confirms the server is idle. Private full
receipts remain under the ordinary-start acceptance directory on G: and the
copied fixture directory under `V:\validation-release-tests`.

Only configuration startup and asset integrity were exercised. No native study
was created in the relocated package. Complete native relocation and distributed
interrupted-run recovery remain separate release acceptance work.

## Native SheetShrink study

A subsequent one-workbook native study used the copied package, the ordinary
SheetShrink starter, the web start endpoint and a new external study directory.
The first attempt completed its five paired captures but failed during retained
image publication. Ordinary image paths and a verbatim-prefixed attempt root
referred to the same files, while the retention check compared their prefixes
lexically. The same attempt exposed a checkout-only report-opener path.

Retention now uses the existing resolved owned-image guard, retaining rejection
of symlinks and outside files. The report opener resolves both supported runtime
layouts. Twelve focused retention tests passed, including canonical-root and
outside-image preservation controls. One explicitly native-chart-dependent test
remained ignored. Four visual-report tests passed, including both launcher
layouts. An initial test invocation targeted the thin executable wrapper and
ran zero tests; it supplies no correctness evidence. The effective retention
test invocation used `cargo test --release -p excel-shrink-cli --lib retention`.

After a guarded release rebuild, a new 132-member package and new study output
completed in 21.188 seconds. Original and SheetShrink each produced five native
images over the five planned regions, with no reported differences under the
targeted methodology. Equal-image retention produced ten integrity records and
five encoded-image deduplications. The package hashes stayed unchanged. The
4 GiB owned-job cap and 8 GiB system reserve remained satisfied. The raw failed
attempt is retained separately. The path-free result receipt is in
[`runtime-native-relocation-20261111.json`](runtime-native-relocation-20261111.json).

This extends acceptance to a small native SheetShrink study. Legacy conversion,
Check Performance, distributed interrupted-phase recovery and final release
inventory remain separate requirements.

## Standalone native preparation

The next full-toolset package contained 133 members, including freshly built
`xlsx-convert.exe` and `excel-check-performance.exe`. The earlier package omitted
the optional conversion executable because the documented build command selected
only the validation and cleanup packages. The release instructions now build all
four application packages before packaging.

From the copied package on V:, one conversion worker processed the existing
generated 25,088-byte legacy XLS fixture in 3.000 seconds. Its nonempty cell
values, formulas and worksheet names matched the earlier XLSX conversion
reference. One Check Performance worker then processed this converted input and
the generated formatted workbook in 7.266 seconds. Native inspection confirmed
`no_optimization_needed` and `optimization_confirmed`, respectively. Values and
formulas matched each input after processing. Both operations exited successfully
with zero failures and no active owned job processes after normal completion.

Each operation had a 180-second deadline, a 4 GiB process-tree commit cap and
8 GiB system physical-RAM and commit reserves. Peak job commit was 592,838,656
bytes for conversion and 674,631,680 bytes for Check Performance. Source fixture
hashes and all runtime member hashes were unchanged. Both Check Performance
outcomes exported successfully, and the transferred package verified again after
copying from V: to G:. The path-free receipt is
[`standalone-preparation-relocation-20261111.json`](standalone-preparation-relocation-20261111.json).

This verifies small native preparation and artifact transfer through the standalone
CLIs. The fixture has no shared editing or macros. Visual equality, larger-corpus
throughput, all interrupted-phase recovery paths and the shared-workbook-specific
conversion fixture remain outside this acceptance.

## Legacy shared-workbook conversion control

Two newly generated BIFF8 inputs exercised the outstanding `xlExclusive` case.
Excel saved each fixture with shared access, enabled change history, changed
A2 from 41 to 42 and recalculated B2's `=A2+1` formula. Reopening the source in
an independently owned Excel instance reported `MultiUserEditing=true`, history
enabled and a 30-day retention setting.

The ordinary standalone converter, with one worker, reported
`shared_editing_removed=true` for both trials. Reopening each resulting XLSX
reported `MultiUserEditing=false`. The first-sheet name, text, numeric value,
formula and calculated value 43 matched. No `xl/revisions/` member was present.
Both source XLS hashes stayed unchanged. Conversion took 3.015 and 2.750 seconds
in these small functional controls. These times are not corpus benchmarks.

Each fixture-generation, conversion and inspection step had its own 90-second
deadline, 4 GiB owned-job cap and 8 GiB physical-RAM/commit reserve. All six
steps completed normally and their jobs had zero active processes afterward.
The first trial used the private fixture recipe. The second exercised the
parameterized public recipe at `tools/tests/shared-workbook-fixture.ps1` with
`-Action create` and `-Action inspect`. Its `-Assets` argument points to the
helper directory emitted by a completed ordinary conversion (`<output>/workers`).
Use fresh workbook and receipt paths for creation. The experiment harness
supplies the outer deadlines and resource guards.

The path-free receipts are in
[`shared-workbook-conversion-20261111.json`](shared-workbook-conversion-20261111.json).
They record the measured Windows fixture-source hash separately from Git's
line-ending normalization. This verifies the intended conversion effect on
these generated inputs. It does not measure visual equality, a BIFF history
record count or the behavior of other legacy features.
