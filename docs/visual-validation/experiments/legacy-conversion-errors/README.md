# Legacy conversion failure receipts

The converter opens an owned Excel instance, disables macros, events and
AutoRecover, ignores read-only recommendations and disables link updating. XLSX
conversion uses SaveAs format 51. The source XLS is copied into an owned staging
directory before Excel sees it.

A synthetic XLS containing text and a numeric value converted successfully.
A second copy protected with a test password failed immediately during Open.
The converter persisted the native message, HRESULT, owned PID and phase as
`excel-open-failure.json`; the category was `password_required`. No empty XLSX
was accepted. The outer Rust watchdog still bounds blocked COM calls, including
cleanup. These two cases exercise ordinary COM completion and failure, not
modal tenant-login or repair dialogs. Those remain separate blocker cases.

The Enron launch uses a 120-second no-progress conversion deadline. A timeout
is an unsuccessful attempt and must never be reported as visual equality.
The run retains converted XLSX baselines and conversion measurements in its
preparation bundles, so restart does not require converting successful inputs
again. Scientific visual validation compares the converted baseline with
Shrink Default. It does not claim binary-XLS versus converted-XLSX equivalence.

To repeat the probe, create a workbook with A1 text and B2 numeric data, SaveAs
Excel 97-2003 format 56, then save a second copy with a password. Run the
production `excel_open.ps1` in a V: staging directory with its existing process
ownership helpers, passing distinct MetricsPath, ProgressPath and OutputPath
for each input. The normal case must create XLSX and metrics. The protected case
must create failure.json with its native reason and no XLSX output.
