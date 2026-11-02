# Conditional workbook states

Native verification on Windows with Microsoft Excel, 2026-11-02. Check Performance
was enabled and invoked independently in every case, on an owned private desktop.
The cases exercise state selection, not corpus-wide runtime estimates.

| Input pair | Excel processes | Observed comparison pairs | Result |
| --- | ---: | ---: | --- |
| Byte-identical static workbook | 2 | 1 | No capture candidates; CP positively reported no optimization needed |
| Static workbook, B1 changed from 1 to 2 | 3 | 2 | O/SD pixel difference detected; O/CP unchanged |
| Same change with B2 formula `=B1*2` | 6 | 9 | Formula retained every rebuild; O/SD pixel difference detected |

The six states are Original (O), Default (SD), Check Performance (CP), and their
independent full-rebuild copies (OR, SDR, CPR). Skipped states have no invented
images. The report records exact input identity, per-parent rebuild eligibility,
and explicit state decisions. Unknown package features preserve rebuilds.

An initial native run retained rebuilds even for static inputs: Excel added path,
revision and AutoRecover metadata during saving. The guard now recognizes those
specific static forms. Other extensions and alternative-content branches still
block omission; focused tests cover both accepted and rejected forms.

## Reproduce

Generate fresh fixtures with `python create-fixtures.py --output <new-directory>`
(requires openpyxl). For each case invoke the repository launcher with absolute
paths and a newly built CLI:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File excel-shrink-cli/office-capture/run-targeted-comparison.ps1 -Binary <cli.exe> -Original <case/inputs/original.xlsx> -Default <case/inputs/default.xlsx> -Output <new-result-directory> -ObservatoryGroup <new-group-directory> -OperationTimeoutSeconds 180
```

Do not pass `-SkipCheckPerformance`. For the zero-candidate test, first save a
static fixture in the same Excel version, then copy that saved file byte-for-byte
as both inputs. Otherwise the first ordinary save can legitimately add candidates.
Native reports and generated workbooks stay outside version control.

Focused Rust tests cover byte equality, static package eligibility, formula/name/
extension blockers and the consumer's required-pair contract. A read-only contract
audit additionally checks these three native reports and retained corpus reports.
Version-two completed results remain valid historical evidence; they are not
rewritten as version-three results.
