# Visual-validation experiments

This tree is intentionally isolated from production code. Experiments may share code
only through `_shared/`; production `src/`, `crates/`, and `tools/` must not depend on
anything here.

The primary entry point is `run.ps1`. In CLI mode it dynamically asks for the experiment
and only then asks for choices relevant to that experiment. The VS Code task can also
select a WPF launcher; it runs the same experiment scripts non-interactively and stores
the last submitted GUI configuration in
`.tmp/visual-validation/launcher-last-config.json`.

Directory layout:

- `_shared/` — experiment-only Office add-in, host implementations, and orchestration.
- `shape-bounds/` — drawing-bound execution-model benchmark.
- `range-image/` — screenshot/bounds/overflow benchmark.

Experiment-specific dependency manifests stay inside this tree. Selecting the Rust
host uses its local `Cargo.toml`; selecting the ASP.NET host uses its local `.csproj`.
Neither dependency set is needed to build or use Excel Shrink itself.
