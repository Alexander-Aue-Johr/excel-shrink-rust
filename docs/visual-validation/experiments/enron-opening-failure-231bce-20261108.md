# Enron opening failure 231bce970fc22d89cf3b

The existing production runner skipped occurrence
`edrm/native_000%2F3.1176077.K0XLQA05Y2QKIBYMRYSE5EJ4TT4CTULPA.5.xls`.
The retained evidence is under the current Enron root at
`evidence/231bce970fc22d89cf3b/targeted-e04c5ae3-0bbc-47e3-9a74-757d635ca43e`.
The raw group error says owned Excel exited during opening. The session evidence
provides a more specific sequence.

| UTC time | Observation |
| --- | --- |
| 17:49:28.680 | Original Excel PID 35712 created |
| 17:49:28.733 | Cleaned Excel PID 51124 created |
| 17:49:58.902 | Cleaned session records Office.js `GeneralException`, `An internal error has occurred.` |
| 17:49:58.932 | Cleaned session begins owned cleanup |
| 17:49:59.105 | Cleaned Excel exit confirmed |
| 17:50:00.022 | Original session records `Resident session finished during workbook startup.` |
| 17:50:00.465 | Original Excel exit confirmed |

Neither session reached add-in readiness or recorded a heartbeat. Original
opening lasted 31.31 seconds and cleaned opening 30.16 seconds. Available-memory
sampling recorded more than 51 GiB. The retained evidence does not establish a
resource exhaustion or workbook-corruption cause.

The earliest recorded failure is the cleaned partner's Office.js exception.
Original shutdown follows it. Thus the top-level exit message alone cannot be
used to conclude that Original independently crashed. Both owned processes were
closed and the scheduler admitted further groups. The occurrence remains an
unvalidated exception. No visual regression or equality verdict follows.

An isolated startup retry and native opening inspection remain required to
distinguish a repeatable workbook/add-in failure from a transient startup error.
Future diagnostic handling should retain the first resident failure and lane
alongside process-exit observations. Preserve the current raw reports when
adding that explanation.
