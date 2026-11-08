# Original resident initialization failure

Workbook identity `9d1992b8fe4ac3c19587` was skipped during the active Enron
fallback run. Its source occurrence is
`edrm/native_000%2F3.1176077.K0XLQA05Y2QKIBYMRYSE5EJ4TT4CTULPA.1.xls`.

The durable record reports that the Original resident exited during opening.
The underlying `launcher-error.json` records Office.js client `one` returning
`GeneralException` with message `An internal error has occurred.` The native
launcher recorded failure after 36.48 seconds and peak job memory of
563,785,728 bytes. Its event log records owned cleanup completing at
2026-11-08T14:49:23.8820212Z.

The retained evidence directory is
`enron-20261107/evidence/9d1992b8fe4ac3c19587/targeted-31b2b91e-d1ae-48de-a1ed-451174e3103b`.
The decisive files are `resident-O/session/launcher-error.json`,
`launcher-result.json` and `launcher-events.jsonl`.

No successful visual comparison was recorded. This evidence does not identify
the underlying Office.js operation or establish workbook corruption. The
failure preceded the controlled preparation-host interruption experiment.
No causal attribution to that experiment or to SheetShrink is established.

## Follow-up

Retain the raw failure and perform a bounded isolated retry when a native test
slot is available. Inspect the client initialization operation and the workbook
window if the error repeats. Keep the outcome unresolved until retry evidence
supports a more specific explanation. The production record was left unchanged.

## Additional opening timeout

Identity `b362146d6aee3d827a2d` subsequently reached an `open_batch` timeout after
180 seconds. Its source occurrence is
`edrm/native_000%2F3.1169592.OHFDNPPXMJZMS0LU24IJMND52TBQUMXEA.1.xls`.
Evidence is retained under
`enron-20261107/evidence/b362146d6aee3d827a2d/targeted-d594e49e-030f-40ef-a511-be80be3f7410`.

Original's launcher recorded cancellation at approximately 178.96 seconds.
The candidate launcher reported that its resident session finished during
workbook startup at approximately 178.78 seconds. That latter message is a
shutdown observation and does not establish the cause of the opening stall.
No specific security, repair or workbook-link diagnosis has been established.
Preserve this failed observation for a bounded isolated retry and window
inspection. No successful visual coverage or SheetShrink regression is inferred.

Identity `59343f0cf251258cecba` also reached the 180-second `open_batch` timeout.
Its source is
`edrm/native_000%2F3.545819.GA0XTHCNKODPLM0NVAN1T0Q21F0VW02RA.1.xls`.
The retained attempt is `targeted-f4e54c8a-9a95-4cd1-bdaf-84440ee3c896` below
that identity's evidence directory. Original reported session termination during
startup after approximately 178.72 seconds; the candidate reported cancellation
after approximately 178.84 seconds. These are timeout teardown observations.
The underlying opening cause remains unresolved and requires isolated inspection.
