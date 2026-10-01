# Visual validation

This directory records the design decisions and minimal reproducible experiments
behind Excel Shrink's visual regression validation.

- `decisions.md` records conclusions that affect the current implementation.
- `experiments/` contains small experiments that provide evidence for those decisions.

Experiments are deliberately kept independent and barebones. Production code and
permanent regression tests belong in the normal source/test tree, not here.

## Running experiments

Reproducible experiments are exposed through VS Code entries prefixed with
`Experiment:`.

Each experiment owns its runner below `experiments/<name>/`. There is intentionally
no shared experiment framework.
