# Add-in startup timing investigation

## Method

One converted Enron workbook and its SheetShrink output were opened together
through the targeted native launcher with Check Performance disabled. The input
was `native_000%2F3.211967.JFIYME2OR41INHZMS5FKDTASFR40PRPBB.1.xlsx`.
Each attempt used fresh staging and fresh isolated WebView profiles. Enron was
paused at the user's request. Each attempt retained its own report, client
metrics and launcher events under the local `sla-opening-20261108` probe folder.

The instrumented native host was a debug build. Browser diagnostics record the
document time origin and initialization milestones. Native process birth times
come from the owned-process cleanup journal. These measurements include add-in
startup and must not be reported as workbook loading alone.

## Registry hypothesis

The Office developer add-in key contained 140 values. Of these, 130 pointed to
missing files. A bounded reversible test backed up and temporarily removed 92
values whose missing paths matched the pipeline's old temporary stage naming.
The values were restored in a `finally` block before the next attempt.

| Attempt | Registry state | Original ready, seconds | SheetShrink ready, seconds |
| --- | --- | ---: | ---: |
| 03 | Existing entries | 34.364 | 35.531 |
| 04 | 92 missing pipeline registrations temporarily absent | 6.066 | 8.191 |
| 05 | All 92 restored | 4.569 | 4.724 |

In attempt 03, the first WebView process appeared about 31.4 seconds after Excel
for both clients. Browser navigation began about 33.3 seconds after Excel. The
workbook identity queries took 36 and 8 milliseconds.

Attempt 05 contradicts attributing the improvement solely to removal of the
stale registrations. Warm caches or another time-dependent influence remain
possible. The experiment does not justify automatic registry cleanup as a
startup performance fix. No permanent registry cleanup was applied.

## Profile isolation

Commit `abb5713c` introduced separate WebView profiles after peer taskpanes
restarted when another group cleaned up a shared browser process. The rationale
and peer experiment are recorded in `docs/visual-validation/cleanup-isolation.md`.
A single-pair startup experiment cannot establish concurrent cleanup safety.

For attempt 07, a separate copy of the Office runtime assets removed both
assignments of `WEBVIEW2_USER_DATA_FOLDER`. Each assignment was replaced with
removal of the environment override, allowing Office to select its default
profile. The ordinary runtime sources retained isolation. The first invocation
(attempt 06) failed before Excel because the copied runtime lacked its sibling
archive cleanup helper. That failed invocation provides no startup measurement.
The helper and its C# companion were included before attempt 07.

| Attempt | Profile policy | Original ready, seconds | SheetShrink ready, seconds |
| --- | --- | ---: | ---: |
| 05 | Fresh isolated profiles | 4.569 | 4.724 |
| 07 | Office default, no environment override | 6.297 | 6.595 |

Attempt 07 completed with the comparison's existing limitations and confirmed
both owned Excel client exits. Disabling the override did not improve startup
in this sequential single-pair comparison. These observations do not establish
cold-start performance, explain the earlier 31-second pre-WebView delay, or
retest independent-group cleanup. Keep profile isolation in the normal runner.

## Opening refactor countercheck

Attempt 08 substituted only `resident.ps1` from the parent of `36ff482c`, before
the opening-step extraction. The native debug binary, input pair, current web
assets, private desktop policy and fresh isolated profile policy were retained.
Attempt 09 immediately repeated the same comparison with the current resident
script and extracted opening functions. The older script lacks the new browser
diagnostic forwarding, but retains the same launch-to-ready measurement.

| Attempt | Opening script | Original ready, seconds | SheetShrink ready, seconds |
| --- | --- | ---: | ---: |
| 08 | Before SLA extraction | 9.550 | 10.081 |
| 09 | Current SLA extraction | 9.880 | 10.566 |

Both invocations returned successfully and confirmed both owned Excel exits.
The current script was 0.33 and 0.49 seconds slower in this sequential trial.
This provides no evidence that the refactor caused the earlier large speedup.
It is one paired trial, without randomization or a controlled cold system, and
does not establish performance equivalence. The focused opening tests also pass
for orchestration order, failures, duplicate clients and add-in readiness.
