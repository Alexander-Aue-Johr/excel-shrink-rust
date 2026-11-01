# Reading the targeted comparison

Start with `targeted_comparison.rs::Comparison::execute`. Its calls describe the
scientific workflow in order. Follow a phase only when its details matter.

| Module | Responsibility |
| --- | --- |
| `targeted_comparison.rs` | Domain phases, candidate selection state and the four captured workbook states |
| `targeted_comparison/candidates.rs` | Cell-XML candidates and separate Original/Default save effects |
| `targeted_comparison/workbooks.rs` | Working copies, opening/saving/recalculating/closing and bounded error cleanup |
| `targeted_comparison/evidence.rs` | Capture evidence for selected regions and compare the four states |
| `targeted_comparison/host.rs` | Named Excel operations, JSON protocol and command deadlines |
| `targeted_comparison/reporting.rs` | Persist outcomes and project them into the Observatory summary |
| `targeted_comparison/telemetry.rs` | Observable process lifecycles, phase transitions and atomic snapshots |

The directory groups responsibilities, not call depth. Small lower-level helpers
remain next to the operations they explain; there is no subdirectory per method
or abstraction layer.

`inspect_save_effects` preserves both pre-save copies, saves the workbooks,
discovers each workbook's own save changes, then includes those candidates.
`SaveChanges.original` remains separate so the result can label Original drift;
it must not be confused with the direct Original-versus-Default comparison.

`ExcelHost` exposes `open_workbook`, `save_workbook`, `recalculate_workbook`,
`close_workbook`, `measure_range` and `capture_range`. Only that module constructs
protocol messages or interprets open/close response fields. Lifecycle observations
remain in `workbooks.rs`; the domain workflow does not parse PIDs from JSON.

Normal closure still requires a confirmed exit. Failure handling releases the
owned host, waits for its processes within the existing bound, writes failure
evidence and reconciles telemetry with the observed exits.

This refactor preserves the existing candidate limits, capture margins, command
order, comparison definitions, output schema and deadlines. It does not complete
the missing chart/layout discovery or post-recalculation candidate expansion,
and it does not switch the corpus runner to this experimental workflow.
