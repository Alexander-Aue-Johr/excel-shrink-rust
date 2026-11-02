# Excel Shrink behavior and reimplementation specification

Review baseline: `eac0300` (2026-11-02), with the scoped corrections recorded in
[review findings](review-findings.md). This is a source-derived specification,
not a claim that every workbook is visually preserved or every source line has
been independently verified. The legacy comparison system still exists; its
behavior is deliberately distinguished from the current targeted workflow.

Current targeted execution policy is `targeted-content-grid-v3`; its conditional
state exclusions and version-two result reuse are documented below. The source
review baseline above remains historical, not an attribution of newer behavior.

## Purpose and reading order

This document set describes observable behavior, preservation decisions, failure
conditions and evidence contracts sufficiently explicitly to guide a separate
implementation and a source-versus-requirements review. Algorithms may be
implemented differently, but changes to the stated decisions must be visible.

1. [Shrinker](shrinker.md): what may change in the XLSX, conditions preventing
   removal, profiles, reference protection, ZIP output and failure behavior.
2. [Targeted validation](targeted-validation.md): workbook states, exact comparison
   pairs, candidate selection, captures, recalculation and interpretation.
3. [Execution and evidence](execution-and-evidence.md): ownership, concurrency,
   resume, result schemas, retention, telemetry and reproducibility.
4. [Review findings](review-findings.md): corrections, known limits, experiments,
   verification performed and questions requiring an owner decision.

For operating commands, retain [the targeted corpus guide](../visual-validation/targeted-corpus.md).
This set complements it rather than creating another launcher or runner.

## Meaning of the statements

- **Implemented rule:** an inspected source path currently makes this decision.
- **Required invariant:** a correct implementation must maintain this property;
  acceptance tests are listed with the responsible source where practical.
- **Deliberate policy:** a requested trade-off, including deliberately unobserved
  exterior formatting; it is not an equivalence proof.
- **Limitation:** behavior is not modeled or not established by the evidence.
- **Experiment:** an observation on specified fixtures and versions, not a
  universal guarantee about Excel or a substitute for a preservation rule.

The shrinker decides what XML to change. Validation decides which observations
to collect and compare. A validation exclusion does not authorize shrinker
deletion; a shrinker safety decision does not establish complete visual equality.

“Unchanged” in a targeted corpus result means no reported difference in the
performed comparisons under the recorded selection policy. It must not be
expanded to “all pixels in the entire workbook were tested.” A report with no
selected cell ranges is possible and must disclose that coverage.

## Terms

| Term | Meaning here |
| --- | --- |
| Input | Original XLSX archive, not an Excel-saved or rebuilt copy |
| Shrink output | XLSX generated using a recorded cleanup profile |
| Empty cell | Structural classification used by cleanup; not a synonym for visually blank |
| Whitespace cell | Recognized shared-string whitespace candidate; not every Unicode blank string |
| Protected reference | Value dependency preventing deferred content removal |
| Style representative | Retained empty cell needed for layout/style inheritance |
| Rebuild | Explicit Excel full calculation rebuild followed by save |
| Candidate | Reason/location for possible capture; not yet a screenshot or a difference |
| Capture region | Selected grid rectangle after content-envelope clipping |
| Difference | Explicit pixel, geometry, chart-presence or rendering-state difference |
| Excluded/skipped | Work not certified by a successful comparison |
| Terminal failure | Attempt ended unsuccessfully; it is not an equality result |
| Process liveness | Windows confirms an exact PID/birth identity exists |
| Add-in heartbeat | Message actually received from the Office.js worker |

## Scope and provenance

The described production paths are the ordinary XLSX CLI pipeline and
`regression study --resident-capture --targeted-comparison`. Profile experiments,
the former full-sheet renderer, hash sweeps, whitespace diagnosis and manual
inspection have separate scopes. Their results cannot be silently relabeled as
the current targeted policy.

Source pointers and test names provide traceability for reviewers. They are not
needed to understand the domain rules, and are not an instruction to reproduce
the same code structure. Inline examples are acceptance cases, not an exhaustive
formalization of SpreadsheetML or the Excel renderer.

For a fresh implementation, first implement the decision tables and invariants,
then use the acceptance cases as differential tests. Preserve separate version
identities for the specification, shrink producer, validator and Excel build.
Compare observable outputs and classifications, not only binary archive sizes.

A **behavioral specification** or **reimplementation specification** is the
appropriate name for this artifact. A **clean-room reimplementation** additionally
describes an organizational separation between analysis and implementation.
This source-informed document set alone does not establish that separation and
does not make claims about licensing or permission to reimplement another product.
