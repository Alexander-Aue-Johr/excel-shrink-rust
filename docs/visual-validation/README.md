# Visual validation

This directory records the design decisions and reproducible engineering experiments
behind Excel Shrink's visual regression validation.

- `decisions.md` records conclusions that affect production.
- `experiments/shape-bounds/` compares drawing-bound execution models.
- `experiments/range-image/` compares screenshot/bounds strategies.
- `experiments/_shared/` contains code shared **only among these experiments**.

Experiments are deliberately isolated from production code. Nothing under `src/`,
`crates/`, or `tools/` should import or require experiment dependencies. When an
experiment chooses a production approach, that implementation is intentionally
copied/reworked into the production tree instead of making production depend on the
experiment.

Sharing inside `experiments/_shared/` is allowed so closely related experiments can
use one Office add-in and comparable host protocol without duplicating those files.

## Running experiments

Use the VS Code task:

```text
Experiment: visual validation
```

It opens a terminal dispatcher. The first choice is `Get picture` or
`Get shape bounds`; later choices depend on that first selection.

Direct scripted entry point:

```powershell
docs/visual-validation/experiments/run.ps1
```
