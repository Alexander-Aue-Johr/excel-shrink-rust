# Portable difference report smoke test

## Question

Can a completed native comparison retain an intentional visible difference and
expose it under a static-site subpath without the Observatory API?

## Fixture and budget

Use the existing `targeted-comparison/make_smoke_fixtures.py` generator with its
default border/font-height counterexample. Copy its two outputs into independent
`a/workbook.xlsx` and `b/workbook.xlsx` source directories. The candidate deliberately
removes a formatting sentinel. This is a detector test with an expected difference.

Allocate one native comparison group, operation and inactivity deadlines of 120
seconds, and a five-minute wall-time budget. Keep production Enron advancing.
Set TEMP, TMP, logs and all source/report paths to a fresh working directory on V:.

Run a standalone configuration with these two sources, `max_processes` of one
and `capture.retain_equal_images` false. Export the completed report with
`deltasieve export REPORT EXPORT`. Serve its parent directory with a loopback-only
static server and open `EXPORT/index.html` through a non-root URL prefix.

## Observed on 2026-11-08

The native run completed one pair with zero failures and one differing region.
The report retained `completed_with_limitations`, preserving its coverage scope.
The export contained two HTML pages, two CSS files, one JS file, four JSON files
and three PNGs. No Excel workbook or executable was exported.

At `/export/index.html`, the browser displayed the retained pair and followed its
relative link to the comparison page. Both evidence images loaded with natural
dimensions of 317 × 2,593 and 317 × 2,561 pixels. The page displayed the pixel
measurements and enlarged hotspot views. The observed count was 59,708 changed
pixels. This count verifies the rendered example, without establishing a general
detector sensitivity or performance result.

Evidence is retained under
`V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\validation-temp\portable-difference-smoke-20261108`.
The test used the development DeltaSieve binary built after `615e9f3`.
Enron continued throughout the experiment. The static server was stopped afterward.

## Remaining publication checks

The current exporter copies run configuration and provenance with local paths.
Public publication requires an explicit metadata/path and credential audit,
including configured processor environment variables and source URLs. Preserve
private provenance separately when preparing a redacted public package. Redaction
must preserve verifiable image integrity and clearly identify altered metadata.

This smoke test covers relative navigation and image inspection for one pair.
Large result lists, accessibility, multi-pair navigation and publication metadata
remain separate acceptance checks.
