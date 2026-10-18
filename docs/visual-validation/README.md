# Visual validation

This component evaluates Excel screenshot capture methods on unchanged workbooks.
The selected method is **Rust Strategy F planning → Office.js / .NET / Selective /
32 MP**, with calibrated geometry, visible context at internal tile boundaries and
strict decoded-pixel comparisons.

| Task | Entry |
|---|---|
| Understand the selected method | [Decision](decisions.md) |
| Inspect published measurements | [Evidence](evidence/index.html) |
| Reproduce the selected comparisons | [Experiment guide](experiments/README.md) |
| Understand scopes, ordering and acceptance | [Protocol](experiments/PROTOCOL.md) |
| Inspect, retain or publish results | [Reporting](experiments/REPORTING.md) |

The evaluation is not the productive original-versus-shrunk regression. Integration
acceptance must check an unchanged pair, known visible changes and failure handling
before a large regression corpus is run. Agreement between capture implementations
is not an independent oracle of visual completeness.
