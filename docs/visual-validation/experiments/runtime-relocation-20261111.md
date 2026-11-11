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
