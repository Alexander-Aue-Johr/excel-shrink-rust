# Visual-validation experiments

For an unresolved workbook/sheet, use [Focused capture reproduction](FOCUSED-CAPTURE.md). The focused GUI and CLI share exact selections and separate phase deadlines; they do not launch a strategy sweep.

> Current runs: see [consolidated capture/report contract](CONSOLIDATED-CAPTURE.md). Output is now central
> at `docs/visual-validation/results/`. Historical paths and monolithic timing notes below
> describe the earlier experiments, not the current safe range-image capture path.


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
- `range-image-height-limit/` — isolated Office.js `Range.getImage()` height-limit diagnostic.

Experiment-specific dependency manifests stay inside this tree. Selecting the Rust
host uses its local `Cargo.toml`; selecting the ASP.NET host uses its local `.csproj`.
Neither dependency set is needed to build or use Excel Shrink itself.

## PNG rendering diagnostics (v2)

- [Height plateau](range-image-height-limit/): calibrated actual-PNG plateau search on the fixed hospital worksheet, adjacent-row repetitions, and separate width controls.
- [Units and markers](range-image-calibration/): deterministic synthetic OOXML, Normal-font and rounding controls, and clipping/scaling evidence.

Both are available in the CLI/WPF launcher and in the dedicated VS Code PNG diagnostics task. They use the ASP.NET host, preserve raw diagnostic artifacts per run, and do not change production capture policy.

## Gridless capture and compact review bundles

See [capture policy and local HTTP host](GRIDLESS-CAPTURE.md) for normalized gridlines,
physical final-PNG crops, metadata-only review bundles, and the Rust desktop HTTP path.

## Configured comparison suite

Use **Visual validation: Run comparison suite** in VS Code. The default `suite.json` runs repeated current candidates; `inventory` covers the experiment families once. See [COMPARISON-SUITE.md](COMPARISON-SUITE.md) for scope, isolation, interpretation and the one-bundle review workflow. Existing individual entry points remain available.

## Bounded strategy decision

Use **Visual validation: Decide strategy (all workbooks)**. See [DECISION-CAMPAIGN.md](DECISION-CAMPAIGN.md) for the predeclared screening/confirmation/all-file plan, baseline-relative deadlines, native scope and generated decision memorandum. This does not run the original-versus-shrunk regression corpus.

## Focused capture repair

Use **Visual validation: Check capture fixes (focused)** for the bounded hidden/narrow-column and geometry follow-up. See [protocol and scope](FOCUSED-CAPTURE-REPAIR.md). It does not repeat the full strategy matrix or change production decisions.

## Final contextual capture qualification

Run **Visual validation: Finalize capture strategy** for the readiness-gated final comparison.
It validates visible-cell capture context and additive geometry first, then runs only
the remaining host/overflow/16–32–64-MP race. See [protocol and limits](CONTEXT-CAPTURE-QUALIFICATION.md).
No historical VBA/GDI+ or synthetic diagnostic matrices are rerun by this profile.
