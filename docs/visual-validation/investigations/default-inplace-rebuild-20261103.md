# In-place Default rebuild verification, 3 November 2026

Base: `experiment/officejs-oversized-capture` at `47161df`.
Strategy: `default-inplace-rebuild-v1`.

- Targeted Rust suite: 104 passed, 10 ignored.
- Capture geometry / diagnosis JavaScript suite: 43 passed.
- Corpus launcher tests: 89 checks passed, including default-off CP and explicit opt-in.
- PowerShell resident ownership tests passed (duplicate and exited-session rejection).
- Default native comparison completed with CP `not_requested`. Original PID 26440;
  SD and SDR both PID 42672. Only two distinct Excel processes. Both required
  comparisons O/SD and O/SDR contained three selected regions.
- Explicit optional CP native comparison completed with `optimization_confirmed`,
  three distinct Excel PIDs, and no explicit Original or CP rebuild.
- Indirect formula fixture: Original and SD-before retained Z20 = A2*2 = 4;
  SD-after had 0 because its A2 input was absent. The direct capture was A1:P21;
  post-rebuild selection expanded to A1:AA21, including late candidate Z20.
- All three native reports passed the production report-consumer contract using
  the read-only retained-report test. No SD-before image is claimed for late regions.

Local artifacts: `.tmp/performance-audit/inplace-20261103/`.
The Python smoke verifiers were updated, but Python is unavailable on this machine;
those scripts were not executed. The actual native inputs and results were checked
with PowerShell/.NET and the Rust production consumer instead.

These small tests verify the execution strategy. They do not establish corpus-wide
stability or fix all prior Resident exits, opening timeouts, or million-row height
budgets. The corpus run was not resumed as part of this implementation.