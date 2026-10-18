# Shared runtime

`office-addin` owns the browser capture code and host endpoints. `suite` resolves
plans, runs isolated Windows workers and builds strict paired summaries. `report`
contains the local workbook inspector and run history. Report I/O is shared by all
entry points. Start from [the experiment guide](../README.md), not internal workers.
