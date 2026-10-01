# Get Shape Bounds experiment

This is the execution-model benchmark for visible drawing bounds used by Excel
Shrink visual validation.

Established implementations remain local to this directory:

- VBA
- Excel-DNA
- VSTO
- PowerShell COM
- Rust COM

Office.js now uses the experiment-local shared add-in infrastructure at
`../_shared/office-addin/` and can be run against either the Rust or ASP.NET host.
Production code does not depend on any experiment project.

Run through the common dispatcher:

```powershell
../run.ps1
```

or directly:

```powershell
./run.ps1 -Implementation run-all -NonInteractive
```

The older base runner is intentionally retained because it is the already-validated
geometry comparison for VBA / Excel-DNA / PowerShell COM / Rust COM.
