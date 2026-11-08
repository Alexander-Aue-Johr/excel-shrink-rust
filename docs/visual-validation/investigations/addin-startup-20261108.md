# Add-in startup timing investigation

## Minimal taskpane and TimeWeaver countercheck

The probe-only Rust command `capture-host inject-addin-probe settings.json`
copies existing ZIP payloads without decompressing worksheets. It changes the
content-type table and one relationship part, then adds the three web-extension
parts. It refuses existing output files and existing web extensions. Two focused
Rust tests cover XML insertion, payload preservation and the alternative
workbook relationship location. The production injector remains unchanged.

`experiments/workbook-switching/hello-world.ps1` uses a separate owned Excel
process, a fresh profile and a 30-second readiness deadline. Its page fetches
configuration and synchronizes the workbook name before posting ready.
Opt-in host setting `probe_access_log` records bounded counts and response status
for taskpane GET, config GET and ready POST. It excludes query strings. The
receipt is initialized before the listener starts. A logging error invalidates
an inference that no request arrived.

The user-supplied TimeWeaver archive was inspected without executing it. Its
`Launcher.cs` registers the manifest under the Office developer registry key.
Its `Workbook.cs` connects taskpanes from `xl/_rels/workbook.xml.rels`, while
our existing injector connects them from the package relationship part.
TimeWeaver uses a fixed add-in identity and launches Excel without `/x`.

Three separate native probes used the same source workbook, minimal page and
fresh per-attempt identity. Their local evidence directories start with
`hello-timeweaver-` in the validation temporary directory.

| Relationship location | Catalog | Taskpane GET | Hello received |
| --- | --- | ---: | --- |
| Workbook | Registry | 0 | No, 30-second deadline |
| Package | Registry | 1, HTTP 200 | Yes, 11.277 seconds after Excel process birth |
| Package | Local FileSystem directory | 0 | No, 30-second deadline |

The successful control also recorded one config GET and one ready POST, both
HTTP 200. The time comes from stored UTC process birth and ready timestamps.
The first receipt's derived `hello_seconds` field was invalid because PowerShell
converted the timestamp using the current culture. The harness now uses a
monotonic stopwatch and names this polling-inclusive observation
`hello_observed_seconds`. Raw timestamps remain available for analysis.

All three probes confirmed owned Excel exit and unchanged input hashes. Registry
probes restored their registrations. The separately opened user inspection
window and host were retained. These observations establish working startup for
the package/Registry control on this machine. They do not explain TimeWeaver's
working behavior on the user's laptop or establish support for all filesystem
catalog arrangements.

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

## Reusing isolated profiles

Attempts 10 and 11 used a separate experimental script copy with one persistent
profile directory per resident lane. Each native invocation returned before
the next began, and both Excel exits were confirmed. This first experiment
preceded the production lease implementation.

| Attempt | Profile use | Original ready, seconds | SheetShrink ready, seconds |
| --- | --- | ---: | ---: |
| 10 | First use | 8.926 | 8.560 |
| 11 | Same isolated directories reused | 8.407 | 9.457 |

Profile reuse worked in this pair but did not consistently improve readiness.
The observation does not establish a startup speedup or independent peer cleanup
safety. The subsequent opt-in Rust pool adds exclusive file leases and quarantines
interrupted owners. Whole-job exit confirmation is required before reuse.

The integrated Rust pool was exercised by attempts 12 and 13. Both used
`slot-000/private` for Original and `slot-001/private` for SheetShrink. Each
attempt confirmed whole-job exit for both residents and left no `in-use` marker.
Both returned successfully with the existing comparison limitations.

| Attempt | Integrated pool use | Original ready, seconds | SheetShrink ready, seconds |
| --- | --- | ---: | ---: |
| 12 | First use | 9.968 | 11.815 |
| 13 | Same slots reused | 8.248 | 9.630 |

Rust tests cover simultaneous exclusive leases, retained cache files, clean
reuse and quarantine after an unconfirmed drop. A native Windows test covers
owned-process termination, confirmation of an empty Job and repeated cleanup.
Six targeted Rust host tests and the resident opening and cleanup scripts pass.
The pool remains opt-in. No active corpus runner was redeployed in these tests.

Attempts 14 and 15 ran concurrently against the same pool. Their four residents
received distinct slots 000, 001, 002 and 003. Both invocations returned
successfully, all four whole-job exits were confirmed, and no dirty lease marker
remained after completion. The readiness times were 13.916/12.016 seconds for
attempt 14 and 14.185/14.927 seconds for attempt 15. This checks concurrent slot
assignment and ordinary cleanup for two small comparisons. It does not reproduce
the earlier four-group visible-reset/upload stress scenario or establish a speedup.
