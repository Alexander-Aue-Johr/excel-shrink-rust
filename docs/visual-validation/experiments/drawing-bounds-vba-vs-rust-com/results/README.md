# Results

The full producer results live directly below this directory:

- `vba.json`
- `csharp-addin.json`
- `powershell-com.json`
- `rust-com.json`
- `vsto.json` when VSTO is available
- `office-addin.json` when the complete Office.js batch succeeds
- `office-addin.partial.json` only when Office.js completed some workbooks before a later batch failure

These files contain the timing measurements. Complete producer results are the source
of `timing-summary.json` and `timing-summary.csv`. Partial Office.js output is kept as
a diagnostic and is not treated as a complete variant by the summarizer.

`geometry-validation.json` contains the geometry agreement result against the VBA
baseline.

Canonical geometry is no longer persisted for every successful producer. Earlier
versions wrote timing-free copies below `compare/`; those files were intentionally
identical whenever two implementations produced the same `right` / `bottom`
geometry, which made them easy to mistake for duplicated timing results.

The summarizer now compares strict-UTF-8, sorted, timing-free geometry in memory.
If every complete variant matches, no geometry-copy directory is produced. If a
variant differs, only diagnostic files for the mismatch are written below:

```text
geometry-mismatches/<variant>/
  vba.json
  <variant>.json
```

Those files contain only `right` and `bottom` and are intended for VS Code diff
inspection. They never contain timing data.

All JSON is decoded explicitly as strict UTF-8. This avoids Windows PowerShell 5.1
mojibake for worksheet names such as `Erläuterung`.

The Rust result retains one semantic caveat until the `excel-com` dependency maps
Excel's `Shape.Visible` `MsoTriState` / `VT_I4` representation correctly. A Rust
geometry match therefore establishes agreement for the current corpus, not full
hidden-shape semantic equivalence.
