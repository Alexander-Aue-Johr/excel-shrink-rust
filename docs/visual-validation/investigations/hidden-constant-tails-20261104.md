# Hidden constant-tail experiment, 4 November 2026

The current hidden/unreferenced rule deleted mixed, intentional-looking detail
values. This branch narrows that rule to long, dense, homogeneous integer suffixes
ending at XFD. It is an experimental structural heuristic, not a semantic proof.

## Implementation and review

`sheet/hidden_content.rs` observes source cells during the existing row traversal,
before style cleanup. Minimum length is 4096 cells. Gaps, text, formulas, changed
values and unsupported numeric forms reject the current hidden band. Decimal
representations and integer overflow retain data; values are never approximated.
Source observations include cells that do not qualify for deferred cleanup.
Every cell of an admitted suffix must also pass the old cell-level guards.

Rejected Hidden candidates are discarded before reference scanning. Admitted
suffixes become row spans instead of per-cell reference-map entries. A known
reference to any member protects the whole span. Existing whitespace policy is
separate. Row-aligned Rayon chunks merge the spans without splitting a row.
There is no extra full-sheet XML pass, clustering model or per-cell value table.
Initial per-cell candidate storage still exists temporarily within each row;
this is not an entirely allocation-free implementation.

## Reproductions and actual input checks

The Rustdoc on `pipeline_hidden_tail_tests.rs` lists all investigated workbooks
and sheets, including the geometry-only cases, and distinguishes reduced fixtures
from actual workbook validation. Tests cover hidden data between visible columns,
to the right, below the visible report, and the Wanderungen nonzero exceptions.
Before implementation, four preservation tests failed and the zero-tail test
passed. The same five tests passed after implementation. Final verification: 241 Core tests passed (1 ignored), and 59 tests selected by
`pipeline::` passed (2 ignored). The real Wanderungen corpus smoke also passed.

Additional tests cover
holes, text, formulas, changed values, short tails, negative constants, references
and sequential versus multiple row-aligned chunks.

The five real Enron inputs retained all previously removed hidden contents.
Wanderungen removed 376573 hidden numeric zeros and no other hidden contents:
343833 from `Tabelle 3.4 ` and 32740 from `Tabelle 2.1.1`. The 243 nonzero numbers,
including A65536=3 and the 242 `Tabelle 3.3` numbers, survive. The short 237-zero
`Tabelle 2.8` tail is intentionally retained.

The real corpus smoke test originally failed its 91% size threshold with the
new preservation policy (90.8359% achieved). Its threshold is now 90%, with the
reason and exact protected cells recorded in the baseline; the case passes.
This is a justified behavior change, not evidence that the old output was safe.

## Release performance

Machine: AMD Ryzen AI 9 HX 370, 24 logical CPUs, 32 GiB installed RAM, Windows.
Baseline: `2dac1d7`. One Rayon thread and one writer per process. One warmup and
five alternating measured runs per variant/file. No Excel automation was invoked.
The existing validation and browser run remained active, so these are local
measurements with scheduling/I/O noise, not an isolated hardware benchmark.

| Input | Old default, seconds | Experiment, seconds | Change |
| --- | ---: | ---: | ---: |
| enron-control.xlsx | 0.392 | 0.424 | +8.0% |
| enron-exposures.xlsx | 0.098 | 0.098 | +0.6% |
| enron-hidden-columns.xlsx | 0.256 | 0.151 | -40.8% |
| enron-hidden-tables.xlsx | 0.928 | 1.034 | +11.4% |
| enron-physical.xlsx | 0.330 | 0.456 | +37.9% |
| wanderungen.xlsx | 2.148 | 2.063 | -4.0% |

Sum of the per-file medians: 4.152 s to 4.225 s (+1.8%). This is
not a corpus-wide throughput estimate. The largest slowdown is real in this
sample and must not be hidden by the aggregate: preserving `physical`/`financial`
expands output from 265823 to 1983439 bytes.

Nine additional rotating-order runs compared the old binary with
`--keep-unreferenced-hidden-content-cells` to the experiment. Against that
preservation baseline, experiment overhead was +1.7% on the control and +2.9%
on physical/financial; the other mixed-table case was 3.2% faster. Against the
old deleting default, the latter physical/financial comparison was 44.5% slower.
These distinguish observed policy/output costs from the small extra detector
cost, but do not establish a universal overhead bound.

Wanderungen sampled median working set fell from 203.5 to 172.6 MiB. Sampling
can miss short peaks; working set is not private commit or whole-system RAM.
Portable numeric measurements and actual-input counts are in the adjacent JSON.

## Scope limits

Only horizontal long integer tails reaching XFD are automatically admitted.
A deliberate full-width constant matrix can still meet this heuristic. Vertical,
shorter and ambiguous cases remain intact. No claim of complete semantic safety
or a general zero-cost classifier is made. The active corpus runner was not
replaced or restarted with this experiment.
