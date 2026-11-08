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
