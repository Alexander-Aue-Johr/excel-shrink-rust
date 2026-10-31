# Check Performance trigger experiment

This experiment inspects the real Office.js runtime and tests native Excel
automation on a disposable workbook. It does not modify the corpus runner,
production worker files, or corpus results.

## Run

From the repository root, with the existing release runner available:

```powershell
python docs/visual-validation/experiments/check-performance-trigger/run.py `
  --output .tmp/check-performance-probe --native --optimize
```

Use a new output directory for every run. Requirements are installed desktop
Excel, Windows PowerShell, Python, and VS Code's bundled Node runtime for the
debugger. No Python packages or .NET SDK are installed by this experiment.

The default optimization attempt uses accessibility, not pointer input. To test
the verified visible fallback, add `--click-optimize`. This moves the pointer
and requires an unlocked interactive desktop. It checks the foreground process
against the experiment's Excel PID before clicking. Do not use the computer
during this brief interaction. The button position comes from UI Automation;
it is not a hard-coded screen coordinate.

`--hidden-optimize` hides Excel immediately before the accessibility action.
It tests hiding an already initialized panel, not startup on a private desktop.
Do not combine it with `--click-optimize`.

## What is tested

The entry point in `run.py` reads as the experiment procedure: prepare the
fixture, open the owned Excel session, inspect Office.js, probe native
optimization, verify saved XML, and record WebView origins. The session context
owns launcher/debugger cleanup and log handles, including failed startup.

`probe-native.ps1` separates connection identity checks, capability discovery,
pane opening, target discovery, action invocation, and completion observation.
`NativeWindow.cs` contains the Windows window-image and pointer-input calls;
`AccessibleAction.cs` contains the MSAA traversal. These are experiment helpers,
not a new production automation layer.

Failure-path checks can run without Excel:

```powershell
python docs/visual-validation/experiments/check-performance-trigger/test_session_cleanup.py
```

1. Copy the production worker scripts into the experiment output directory and
   inject one additional diagnosis operation into that copy.
2. Open a small workbook with values in A1:A20 and a surplus formatted empty
   cell at Z10000. Use the normal add-in injection and resident launcher.
3. Attach to this WebView2 through a process-local, loopback debugging endpoint.
   Pause at a real JavaScript `debugger` statement. Inspect Office, Excel,
   workbook, worksheet, range, application, ribbon, and document objects through
   CDP, then resume. Unknown methods and property getters are not executed.
4. Attach COM to the exact owned Excel process, checking PID, process start time,
   and workbook name. Query possible ribbon command IDs and execute the
   recognized `CheckPerformance` command, separately from optimization.
5. Inspect the native performance pane. Try the unique Optimize all button's
   UIA action, or its MSAA action when UIA offers no invocation pattern.
   Pointer input is a separate, explicit experiment option.
6. Save copies before and after the action. Preserve pane events, a window
   image, runtime inventories, and debugging evidence. The resident owner
   performs bounded cleanup of its own session.

The before-copy is made after an ordinary Excel save, avoiding a comparison
between Excel's normal XML rewrite and the optimization action.

## Findings

Tested with Excel 16.0.20527.20088 on this Windows machine. These are observations
of this installation, not a promise about every Excel version.

| Route | Observed result |
| --- | --- |
| Office.js runtime inspection | Real breakpoint reached; no performance/optimization method found on the inspected objects. This does not prove absence of a private API. |
| `CommandBars.ExecuteMso("CheckPerformance")` | Accepted, including while `Application.Visible` was false; Workbook Performance pane subsequently observed. Command return alone does not prove hidden optimization. |
| Guessed OptimizeAll/OptimizeSheet/OptimizeWorkbook command IDs | Not recognized. |
| UIA Optimize all button | Found, but reported no supported invocation patterns. |
| MSAA default action, visible | Returned without error, but did not change the worksheet XML. Not a successful trigger. |
| MSAA default action, hidden after panel initialization | Failed with HRESULT E_FAIL. No successful hidden optimization established. |
| Visible pointer input on the discovered button | Success: pane reported "Workbook optimized" and "2 cells were optimized"; Z10000 was removed from saved worksheet XML. |

The pane exposed native NetUI accessibility elements. The experiment's WebView2
debugging endpoint listed only Excel Shrink Capture, not a separate performance
page. That is insufficient evidence to classify the panel's complete internals,
but does not support treating it as an accessible ordinary Office.js add-in.

Raw reports are local under `.tmp/check-performance-trigger-*`. The compact
`findings.json` records the verified results without session URLs or tokens.

## Implications

Keep workbook operations in Office.js and isolate the performance trigger in
a native adapter. Opening the pane is currently simple; completing optimization
requires a separate, observed action. Never report command acceptance as
optimization success. A production adapter needs bounded waits and explicit
completion/error reporting.

Hidden windows are not categorically incompatible with accessibility, but no
working hidden Optimize all path has been established here. The verified pointer
fallback requires visibility and must not be silently used on an unattended
hidden corpus run. Private desktops and minimized windows require separate tests.
