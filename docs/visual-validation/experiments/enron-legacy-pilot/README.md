# Enron legacy-XLS pilot and launch settings

The supplied `219f5e4e42c4397a98d8.zip` contains 20,872 XLS entries, comprising
20,256 EDRM and 616 NUIX files. Other legacy formats and documentation are not
part of this XLS selection. The V: archive copy matches the Kingston source.
Archive SHA-256 is
`45d3eb7d3a2cda5b94e2903f70ad064583220f35b94eba2c6a3f3d1421c00b3b`.

Twelve inputs were selected at size quantiles 5%, 25%, 50%, 75%, 90% and 99%
inside each source directory. All converted to XLSX and were processed with
Default. Nine completed visual comparisons without differences, two produced
identical shrink packages, and one required repair when reopening the converted
baseline. A direct Excel opening without the injected add-in showed the same
repair prompt. This attempt remains skipped. The pilot did not establish that
conversion preserves every feature of the binary XLS input.

New runs enable `sqlite_metadata` and disable `retain_equal_images` in launcher
settings. `targeted_check_performance` stays false. Targeted preparation selects
Default only. Existing successful converted baselines, conversion measurements
and shrink outputs stay in prepared bundles. Restart requires the ordinary
prepared-build approval for a changed executable, even when only validation
changed. Keep the same corpus identity and output root when resuming.

The corresponding direct CLI options are

```text
regression study --request-file V:\path\request.json
  --output-root V:\path\enron-run --download-root V:\path\downloads
  --run-corpus Enron --resident-capture --targeted-comparison
  --targeted-skip-check-performance --retain-equal-images false
  --chart-capture-mode deterministic --comparison-workers 4
  --control-room --control-room-port 18777 --order size-asc --hide-excel
```

Set TEMP, TMP and EXCEL_SHRINK_LOG_DIR to V: before launch. Set
EXCEL_SHRINK_SQLITE_METADATA=1. The launch wrapper configures these settings for
the monitored run. Conversion no-progress timeout is 120 seconds and the rebuild
timeout is 180 seconds. Authentication, file-block, repair and save failures
remain unsuccessful attempts with their native messages. Macros and AutoRecover
are disabled. Successful comparison means converted-XLSX versus Shrink Default,
not binary-XLS versus XLSX equivalence.

The live controls separate preparation concurrency from visual groups.
Geometry batches of 256 ranges and capture batches of eight ranges with a
256 MP combined allowance were separately tested. Four visual groups are kept.
Eight preparation workers initially reached roughly 2,500 files/hour on small
inputs. Sixteen provided no improvement and produced a COM activation failure
and long waits. The run was stopped cleanly, successful results retained, and
that infrastructure failure scheduled for retry at lower concurrency. These
observations use different successive inputs and are not a controlled benchmark.
Four preparation workers are the restart setting while stability is checked.
