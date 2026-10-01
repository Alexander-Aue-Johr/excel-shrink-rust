$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'excel_combined_overflow.ps1')
function Write-ProgressLine($message) {}
function Release-OverflowCom($obj) {}
function Ensure-OriginalWorkbook { $script:Opened++ }
function Test-CaptureGeometry($cache,$range) { return $false }
function Test-CompatibleCachedPair($a,$b) { return $false }
function Test-OverflowStrip { throw 'Separate strip captures must never run' }
function Worksheet-RangeByBounds($sheet,$row,$column,$rows,$columns) { return [pscustomobject]@{Rows=$rows;Columns=$columns} }
function Capture-WorksheetPairWithoutOverflow { $script:Skipped++;return 'ordinary capture' }
function Capture-RangeImageSetWithPartialCache($left,$right,$leftRange,$rightRange) {
    $script:Captured.Add([int]$rightRange.Columns)
    return [pscustomobject]@{capture_info=@{columns=$rightRange.Columns}}
}
function Invoke-CombinedOverflowAnalysis($capture,$index,$base,$end,$extra) {
    $script:Budgets.Add([int]$extra)
    if($extra -eq 1){return [pscustomobject]@{done=$false;next_column=110;next_extra=8}}
    return [pscustomobject]@{done=$true;keep_column=107;capture=[pscustomobject]@{capture_range='A1:DC22'}}
}
$script:Opened=0;$script:Skipped=0
$script:Captured=New-Object 'System.Collections.Generic.List[int]'
$script:Budgets=New-Object 'System.Collections.Generic.List[int]'
$worksheets=[pscustomobject]@{}
$worksheets | Add-Member ScriptMethod Item {param($index) return [pscustomobject]@{Name='test'}}
$original=[pscustomobject]@{Worksheets=$worksheets}
$right=[pscustomobject]@{Name='test'}
$limit=[pscustomobject]@{max_row=22;max_column=2;overflow_first_column=3}
$result=Capture-WorksheetPair 1 $right $limit $null $null
if(($script:Captured -join ',') -ne '3,110' -or ($script:Budgets -join ',') -ne '1,8'){throw 'Combined capture did not use Rust expansion plan'}
if($result.capture_range -ne 'A1:DC22' -or $script:Opened -ne 1){throw 'Final analyzed capture was not reused'}
$limit.overflow_first_column=2
[void](Capture-WorksheetPair 1 $right $limit $null $null)
$limit.max_column=16384;$limit.overflow_first_column=16384
[void](Capture-WorksheetPair 1 $right $limit $null $null)
if($script:Skipped -ne 2 -or $script:Captured.Count -ne 2){throw 'Hidden suffix or XFD must skip overflow capture'}
Write-Output 'Combined capture tests passed: Rust-planned expansion, reuse, hidden suffix, XFD and no strip captures.'
