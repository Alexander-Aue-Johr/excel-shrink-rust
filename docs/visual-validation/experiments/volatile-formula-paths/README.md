# Volatile dependency paths

This diagnostic experiment checks whether a small reference tracer can explain
the observed Cluster2 RAND case without rebuilding Excel's formula engine.
It is not connected to validation decisions or the Observatory yet.

The prototype follows direct A1 cells, absolute coordinates, quoted worksheet
names, ranges, literal A1 INDIRECT calls and translated shared formulas. Ranges
visit existing formula cells instead of allocating every coordinate. It finds one shortest path to RAND,
RANDBETWEEN, RANDARRAY, NOW or TODAY and returns the exact call span for a UI
highlight. It ignores references inside string literals, detects cycles and
reports unsupported computed dynamic references explicitly. A search budget limits
diagnostic traversal, never capture coverage or the scientific comparison.

Run its tests with Python's standard library:

```powershell
python -m unittest discover -s docs/visual-validation/experiments/volatile-formula-paths -p test_trace.py
```

The CLI takes an XLSX path, worksheet name and cell coordinate. Source cached
values are labeled as such: they are not the actual values from rebuilt capture
states. Finding a path cannot prove that every visual difference was caused by
that function. Production integration must preserve compared-state value
evidence and keep unresolved/mixed findings prominent. No Python runtime is
being introduced as a production dependency.

## Observed Cluster2 case

For `Cluster2_DEA data_May2018_b50550fde489.xlsx`, the source contains 1,537
formula-bearing cells. The experiment finds a supported path to a random/time
function for 722 of them; 815 have no such supported path, with no unresolved
branches in this sample. Loading and tracing all cells took 0.381 seconds in
one local run, without opening Excel. This is a feasibility measurement, not a
throughput benchmark or an attribution of all captured pixel differences.

One observed path is `Fig. 5a Aux.ARWH!P7 → C7 → RAND()`. P7's shared formula
originates at P5, and C7's shared RAND formula originates at C4. Its translated
formula references C7 directly among other inputs, so no dynamic-reference
evaluation is needed. The source cached values differ from independently
rebuilt values and must not be presented as the captured values.
