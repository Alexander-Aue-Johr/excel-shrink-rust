# Implementation baseline for the approved delivery plan

The active Enron runner was verified through process PID 17640, its launch receipt, and the live `/api/summary` endpoint on port 18777. Its executable is pinned to commit `abc9e79` and binary SHA-256 `11158836206f7bb9d0898d7851e79190c50f9ef58d67b8d974505bf7d93e09f3`.

The run root is `V:\excel-shrink-paper\external\excel-shrink-rust\.tmp\enron-20261107`. Its launch receipt retains the complete argument list and request path. Recovery must use that receipt, the same output root, and the recorded compatibility settings. Verify the process identity and terminal state before recovery.

## Corpus and execution

The selected archive contains 20,872 inputs. Preparation is terminal for all inputs, with 20,503 prepared. The visual scope is 16,799 workbooks. At this observation, 4,080 were compared, three were skipped, four were active, and 12,716 remained. The rolling throughput was 324 completed workbooks per hour. These figures are a baseline snapshot.

The active configuration uses Default, four comparison groups, resident capture, deterministic chart selection, hidden Excel, disabled Check Performance, and disabled equal-image retention. The primary execution strategy is paired in-place rebuild. Live reports identify capture policy `targeted-content-grid-v9`.

The current corpus guide contains historical v3/v8 policy descriptions and a multi-instance description that differs from this active strategy. Reconcile those passages during the contract review using the current implementation and retained evidence.

## Standalone branch integration

The local remote-tracking branch `origin/codex/sheetscope-validation` provides a web configurator, standalone coordinator, source acquisition, result reuse, and portable-report components. Its library target shares `src/main.rs` with thin binary adapters. It currently calls the tool SheetScope and exposes a `sheet-scope` executable.

The branch comparison spans 176 files. Its tree predates current SQLite documentation, capture experiments, process metric presentation, inspection windows, and other changes on the active branch. Integrate the relevant standalone components with an explicit compatibility review of those current facilities.

The standalone workflow supports external processing commands, which require review against the requested native SheetShrink integration. Its bounded monitoring and static report components are candidates for reuse.

## Capacity and continuity

Initial free capacity was approximately 85 GiB on C:, 317 GiB on V:, and 1,374 GiB on G:. Prefer V: for working data and evaluate G: for overflow if needed. Preserve the system-drive reserve and record each owned working directory.

The pipeline continues during development. Native experiments use the approved bounded test windows and confirm resumed Enron progress after any interruption. Outstanding visual findings remain subject to their retained evidence and individual review.
