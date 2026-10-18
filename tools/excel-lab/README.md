# Native Excel reference backend

The VBA modules, `ExcelInterop.ps1`, `Plan.ps1` and `worker.ps1` implement the native
reference adapters used by [Visual validation](../../docs/visual-validation/experiments/README.md).
Start individual bitmap, EMF, memory-pair or row-height experiments in that GUI.
The controller builds or reuses the macro host, preserves native timings and captures
only private working copies. Macro/security prerequisites are not disabled by a test.

`native/` contains the Rust image-comparison reference. `tests/pure.ps1` checks pure
PowerShell logic; it does not launch the removed UI or prove Excel/clipboard behavior.
The VBA/helper manifest is generated build state, not experiment input evidence.
