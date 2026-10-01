# Drawing bounds execution-model benchmark

## Goal

This is an engineering benchmark for the Excel Shrink visual-regression pipeline.
It is not a paper experiment.

The goal is to find the fastest reliable execution model for collecting the drawing
bounds that are needed before screenshots are captured from the original and shrunk
workbooks.

The extended runner can compare six candidates:

1. VBA inside Excel.
2. C# / Excel-DNA inside Excel.
3. C# / VSTO inside Excel.
4. Office.js inside Excel's web add-in runtime.
5. external Windows PowerShell through Excel COM Automation.
6. external Rust through Excel COM Automation.

`run-experiment.ps1` runs the original four candidates. `run-all.ps1` runs those
four and then attempts VSTO and Office.js.

## Geometry

For every visible drawing, the implementations read or batch-load:

- `Left`
- `Top`
- `Width`
- `Height`
- `Rotation`

They calculate the same rotation-aware axis-aligned envelope and retain only the
maximum `right` and `bottom` point for each worksheet.

Rust currently has one explicit dependency caveat: `Shape.Visible` filtering is
disabled until the `excel-com` dependency handles Excel's `MsoTriState` / `VT_I4`
representation correctly. A Rust match therefore establishes the resulting geometry
for this corpus, not full hidden-shape semantic equivalence.

## Timing scope

VBA, Excel-DNA, PowerShell COM, Rust COM, and VSTO time the drawing-bound operation
inside each worksheet. Workbook open/close and result serialization are outside
those per-sheet timers.

Office.js uses the API idiomatically: worksheet shape collections are queued and
loaded in a batch. Its `_meta.geometry_total_ms` measures the batched
`context.sync()` that retrieves the shape properties plus the local
rotation-envelope calculation. Add-in registration, web-host startup, task-pane
startup, workbook open/close, and result serialization are outside that timer.

`timing-summary.json` and `timing-summary.csv` aggregate these producer measurements.

## Result files

Full timing-bearing producer output is written directly below `results/`:

```text
results/
  files.txt
  vba.json
  csharp-addin.json
  powershell-com.json
  rust-com.json
  vsto.json             # when available
  office-addin.json     # when the full Office.js batch completes
  office-addin.partial.json  # only after a partial Office.js batch failure
  geometry-validation.json
  timing-summary.json
  timing-summary.csv
```

Successful geometry comparisons are no longer duplicated into `results/compare/`.
The summarizer compares canonical, strict-UTF-8, timing-free geometry in memory.

Only an actual mismatch creates diagnostic files:

```text
results/geometry-mismatches/<variant>/
  vba.json
  <variant>.json
```

Those diagnostic files contain only `right` and `bottom`. Timing always stays in the
full producer result files.

This also avoids a previous source of confusion: identical files below `compare/`
were expected when geometry matched; they were not duplicate performance
measurements.

## UTF-8

All comparison and summary JSON is decoded explicitly as strict UTF-8. Do not use
Windows PowerShell 5.1 `Get-Content` without an explicit encoding for these files;
that can turn worksheet names such as `Erläuterung` into mojibake.

## VBA

The VBA host is cached below:

```text
<repo>/.tmp/visual-validation/drawing-bounds-vba-vs-rust-com/vba-host/
```

Excel only needs programmatic VBA-project access when that host has to be created or
rebuilt:

**Excel -> Options -> Trust Center -> Trust Center Settings -> Macro Settings ->
Trust access to the VBA project object model**

## C# / Excel-DNA

The Excel-DNA candidate targets `net8.0-windows`. The runner uses a Windows .NET 8
SDK from `PATH` or the existing repo-specific user-local toolchain cache below
`%LOCALAPPDATA%\excel-shrink-rust\repo-toolchains\...`.

NuGet access is required on the first Excel-DNA build.

## C# / VSTO

The VSTO candidate targets .NET Framework 4.8 and requires the Microsoft VSTO/Office
build tools. `run-all.ps1` skips it when those build tools are not installed.

## Office.js without Node

The Office.js candidate intentionally has no Node, npm, npx, Deno, or
`node_modules` dependency.

The task-pane files are served from a tiny ASP.NET Core/Kestrel host that targets
.NET 8. This reuses the Windows .NET 8 SDK already needed by the Excel-DNA
experiment. The web host is experiment infrastructure; it is outside the measured
Office.js geometry timing.

On the first Office.js run, the runner checks for a trusted ASP.NET Core development
certificate and, if needed, invokes:

```text
dotnet dev-certs https --trust
```

Windows can show a one-time trust confirmation.

For Windows desktop sideloading the runner mirrors Microsoft's developer
registration directly under the current-user Office registry:

```text
HKCU\Software\Microsoft\Office\16.0\Wef\Developer
```

The manifest ID points to a stable manifest copy below the user's local application
data directory. The registration is persistent by default so Excel continues to
regard the development add-in as sideloaded across benchmark runs. Pass
`-TemporaryDeveloperRegistration` when an ephemeral registration is explicitly
wanted.

Registration alone only makes the development add-in available to Excel. Each
temporary workbook copy is therefore also tagged with Office Open XML
web-extension/task-pane parts. The embedded reference uses the registered manifest
ID, sets `Office.AutoShowTaskpaneWithDocument=true`, and sets task-pane
`visibility=1`.

The manifest contains a `ShowTaskpane` action whose `TaskpaneId` is
`Office.AutoShowTaskpaneWithDocument`. A one-time trust prompt can appear on a
machine where the development add-in has not been activated yet. Once activated,
normal batch runs should not require clicking the ribbon button for every workbook.

The task pane references Microsoft's Office.js CDN endpoint,
`https://officeapis.public.onecdn.static.microsoft/1/office.js`. A separate local
`bootstrap.js` resource is loaded first so the diagnostic log can distinguish task
pane activation, browser-script execution, Office.js loading, `Office.onReady()`,
and the Excel API operation.

The runner launches each tagged temporary workbook through the Windows shell, which
matches Microsoft's desktop sideload tooling. The task-pane JavaScript submits the
result and then closes that benchmark workbook itself. The runner waits briefly for
the WebView/task pane teardown before launching the next document. A best-effort
cleanup closes only the exact temporary workbook if a launch times out.

The first workbook gets a separate activation window because Office can require one
explicit trust/consent action when a sideloaded add-in is activated for the first
time. After that one-time security boundary, the tagged workbooks are expected to
auto-open the task pane without a ribbon-button click for every workbook. The normal
per-workbook timeout remains 15 seconds, and completed measurements are preserved in
`office-addin.partial.json` if a later workbook fails.

On Windows, Office Add-ins served from `https://localhost` can require a localhost
loopback exemption for Microsoft Desktop App Web Viewer on some configurations. The
runner reports the state but does not warn merely because the historical exemption
is absent; current WebView2 setups can work without it. Pass
`-ConfigureWebViewLoopback` only when localhost loading actually fails and the
exemption is needed.

For host-level manifest/loading diagnosis, pass `-EnableRuntimeLogging`. The runner
temporarily enables Office runtime logging and restores the previous registry state
afterwards. Runtime logging affects performance, so it must not be enabled for final
benchmark timing runs. Diagnostics are stored under
`results/office-addin-diagnostics/`.

The shared manifest/runtime check requires `ExcelApi 1.11`. The drawing-bounds APIs
themselves need less, but 1.11 is used so the task pane can close each benchmark
workbook after submitting its result and the shell-launched batch can continue
unattended.

## Rust helper

Build the Windows Rust COM helper from WSL:

```bash
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
./build-rust-com.sh
```

The build uses `cargo build --locked` and records a source fingerprint next to the
Windows executable.

## Run

From this experiment directory in WSL, run all available candidates:

```bash
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File ./run-all.ps1
```

Run only the original four candidates with:

```bash
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File ./run-experiment.ps1
```

The VS Code task **Experiment: drawing bounds execution models** runs `run-all.ps1`
so that the implementation candidates are compared in one command.

## Interpretation

Geometry equality is a correctness gate; lower timing is the implementation-selection
signal.

The current engineering evidence makes VBA the practical default candidate for
drawing bounds: it is fast, direct, and substantially easier to automate than the
Office.js task-pane lifecycle. Office.js remains useful as a reference implementation
rather than a reason to add web-add-in infrastructure to the production path solely
for geometry collection.

There is one Office.js operation that can still change the screenshot architecture:
`Range.getImage()` directly renders a range as a Base64 PNG. The sibling experiment
[`../range-image-vba-vs-officejs-getimage/`](../range-image-vba-vs-officejs-getimage/)
compares that complete PNG-producing operation against the VBA
`Range.CopyPicture -> Chart.Paste -> Chart.Export` path.
