# Targeted comparison cards: behavior and acceptance

This view belongs to the bounded Original/Default targeted comparison, not the
ordinary corpus capture scheduler. It keeps that distinction visible. Check
Performance is enabled as a third lane on a separate saved Original copy, using
the verified private-desktop native trigger. The CLI can explicitly omit it with
`--skip-check-performance`.

## Starting a comparison

In the Atelier, **Targeted recheck · Original / Default** extracts two fresh
copies from one prepared bundle and starts the existing CLI workflow on an
owned private desktop. It does not pause or change the corpus queue. No prepared
workbook or earlier result is overwritten. A missing Default variant produces
an explicit error.

The public CLI entry remains:

```powershell
excel-shrink-cli.exe regression targeted-compare `
  --original C:\inputs\original.xlsx --default C:\inputs\default.xlsx `
  --output C:\results\new-directory `
  --observatory-group-dir C:\study\control-room\groups\group-id `
  --workbook-label "Workbook to inspect.xlsx" `
  --operation-timeout-seconds 120 --skip-check-performance
```

The example explicitly omits CP and retains the existing visible-COM behavior.
CP is enabled by default and requires the private-desktop launcher. The
Observatory uses `office-capture/run-targeted-comparison.ps1` to run all three
lanes privately. The host verifies the requested desktop before showing Excel.

This experimental COM capture uses the Windows window station's shared
clipboard. External clipboard activity or simultaneous COM rechecks can fail a
capture. Sequence and owning PID are checked, so an unrelated image must never
be accepted as evidence. The ordinary Office.js corpus path does not use this
clipboard mechanism.

## Process cards and signals

- One small card per available workbook lane, with its real PID, current
  worksheet/range and capture progress. Explicitly omitted CP has no pretend process
  card; its unperformed steps remain visible in the diagram.
- The Excel-style flat workbook/X icon is neon green. It pulses only while
  opening, stays illuminated while the workbook is open, and fades after
  confirmed process exit. A close request alone does not start the fade.
- The XML document/code icon is violet. It represents the Rust host inspecting
  this workbook's XML. Start and finish timestamps preserve short activities
  across UI refreshes; repeated snapshots do not restart their afterglow.
- Both symbols and their neon halos are drawn by a signed-distance fragment
  shader. All icons share one 128×128 WebGL2 context. Offscreen/hidden cards do
  not animate. Reduced-motion settings suppress continuous icon animation.
  An explicitly labelled text fallback handles unavailable WebGL.
- The existing heartbeat renderer is reused in heartbeat-only mode. The
  overview's full monitor retains its capture and phase tracks.
- This COM host has no Office.js heartbeat stream. Its cards instead label real
  **PID-liveness samples**, checked against process creation time. These are
  neither Excel responsiveness nor work progress. No heartbeat is inferred from
  a phase, capture count, or mere PID assignment.

Card nodes, icon canvases and heartbeat canvases are retained across snapshots
and across the move into completed history. Only changed values and genuinely
new/removed cards affect the DOM.

## Progress and workflow diagram

The candidate count measures regions selected from changed cell XML and
ordinary-save changes. Each lane captures each region before and after its
rebuild. A growing queue increases the denominator without discarding completed
work. Unknown totals remain indeterminate; a failed run never gets an artificial
100% bar.

The diagram has arrow-shaped column headers and disabled status checkboxes for:

1. Discover XML candidates.
2. Open the workbook copies.
3. Save the open copies normally.
4. Add save-only XML changes to the candidate queue.
5. Run Optimize all on the CP copy and require positive completion.
6. Discover optimization changes against the CP save baseline.
7. Capture candidate regions before rebuilding.
8. Full rebuild: Default, CP, then Original as an independent control.
9. Capture those same regions after rebuilding.
10. Compare pixels and rendered geometry.
11. Close each owned Excel process and verify exit.

Rows distinguish Original, Default and Check Performance (or explicitly omitted CP). Completed, running,
pending, skipped, failed and unavailable steps are distinct. Column/cell
tooltips explain the purpose of the control. The view also spells out the
three comparison pairs:

- Original-before ↔ Default-before: direct shrink effect.
- Original-before ↔ Default-after: shrink result after rebuild.
- Original-before ↔ Original-after: original calculation-drift control.

The column definitions come from execution telemetry rather than a separately
maintained UI phase list. Coverage limitations remain visible in the card and
are preserved in `report.json`.

## Completion and history

A report is terminal only after explicit completion or failure reporting.
Available Excel lanes must also have confirmed exits before their workbook card
moves into completed history. Failed/unconfirmed cleanup remains visible as
active attention work. Job cleanup is restricted to this comparison's processes.

The default completed list shows three latest records. **…** reveals more.
**Full screen** shows the retained list with vertical scrolling; Escape returns.
The host returns all active targeted comparisons and the latest 100 completed
ones. Merely selecting another workbook does not discard this history.

`telemetry.json` is the run-local snapshot. The optional group snapshot remains
`targeted-comparison.json`. A dedicated `targeted-comparisons/` registry beside
`groups/` supports `GET /api/targeted-comparisons` and page reloads without
scanning every corpus group's directory. Registry entries contain compact
telemetry; evidence images and the scientific report stay in their own run.

## Source responsibilities

- `regression/targeted_comparison.rs`: readable sequence of domain phases.
- `targeted_comparison/candidates.rs`: bounded XML candidate discovery.
- `targeted_comparison/evidence.rs`: capture evidence and compare required pairs.
- `targeted_comparison/host.rs`: command deadlines, host protocol and PID identity.
- `targeted_comparison/telemetry.rs`: typed lifecycle/step states, timestamps,
  independent liveness sampling and serialized atomic publication.
- `capture_host/control_room/targeted.rs`: registry queries and explicit launch.
- `control-room/targeted-panel.js`: retained cards, flow diagram and history.
- `control-room/process-icon-shader.js`: shared icon renderer.
- `control-room/targeted-model.js`: deterministic UI decisions.

The host receives a bounded command deadline. Closing has a separate maximum
of 15 seconds. Destructors do not attempt blocking COM calls. The PowerShell
host's kill-on-close job releases its own Excel processes after transport
failure; failure reporting gives their exits a bounded confirmation interval.

## Acceptance checks

| Requirement | Evidence |
| --- | --- |
| True shader rendering and bounded contexts | Headless Edge runs the actual GLSL; at most two shared contexts for icons and heartbeat traces. |
| Stable canvases | Browser test preserves element identity across telemetry updates and completed-card relocation. |
| Heartbeat only | Browser checks absence of capture/progress legend tracks in these cards. |
| Honest activity and progress | Model tests cover opening/open/closed states, millisecond XML work, afterglow, stale snapshots, missing beats and growing queues. |
| Checkbox flow | Browser checks the complete phase-by-lane matrix, including native optimization and its XML inspection. |
| Completed history | Browser checks the initial limit, expansion and fullscreen scrolling. Host unit test keeps old active runs and late finishers. |
| Real comparison | Private-desktop smoke test captures two candidates in four states; it detects the injected font-height difference and no original-recalc drift. |
| Failure cleanup | Fault-injection smoke test fails during capture and requires a terminal failure with both Excel exits confirmed. |
| Actual Observatory launch | HTTP test starts the private runner from a prepared bundle, checks real comparison results and verifies the bundle hash is unchanged. |
| Bounded command wait | HTTP harness also runs a deliberately silent command host with a one-second deadline; it must finish as a reported failure. |

The comparison still has scientific limits: at most 64 candidate regions; no
complete drawing/chart/style/layout dependency analysis; no new candidates
introduced only by a rebuild; no reopened-after-save check. The bounded in-memory index rejects worksheet XML over
64 MiB or 250,000 cells. These limits fail explicitly or remain recorded; none
are represented as whole-workbook visual equality.

## WIP history split

The original `2d04929` combined four concerns. They were split without changing
the resulting tree, preserving the original author:

| Commit | Concern |
| --- | --- |
| `65ba8cd` | Inherited-formatting tail cleanup |
| `b87fdb1` | Row-geometry experiments |
| `5b9843e` | Standalone targeted comparison |
| `38bc484` | Initial Observatory telemetry display |

Subsequent experiment commits were replayed unchanged. The former history,
including `2d04929`, remains on
`backup/officejs-before-process-cards-20261101` on origin. The later font-tail
investigation's known shrinker findings were not silently fixed by this UI work.
