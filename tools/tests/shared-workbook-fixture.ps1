param(
    [Parameter(Mandatory=$true)][ValidateSet('create', 'inspect')][string]$Action,
    [Parameter(Mandatory=$true)][string]$Assets,
    [Parameter(Mandatory=$true)][string]$WorkbookPath,
    [Parameter(Mandatory=$true)][string]$ReceiptPath
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $ReceiptPath) { throw 'Receipt already exists' }
Add-Type -Path @((Join-Path $assets 'CaptureProcesses.cs'), (Join-Path $assets 'CaptureDesktop.cs'))
Import-Module (Join-Path $assets 'CaptureSetup.psm1') -Force -DisableNameChecking
$owned = New-Object CaptureProcesses
$desktop = [CaptureDesktop]::new()
$checking = $null
$excel = $null
$workbook = $null
try {
    $seed = Join-Path (Split-Path -Parent $ReceiptPath) ('seed-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
    Copy-Item -LiteralPath (Join-Path $assets 'seed.xlsx') -Destination $seed
    $process = $desktop.Start((Find-ExcelExecutable), ('/x /automation ' + (Quote-Argument $seed)), $owned)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($null -eq $checking -and $watch.Elapsed.TotalSeconds -lt 30) {
        $checking = [CaptureErrorChecking]::TryDisable($process, $desktop.Name)
        if ($null -eq $checking) { Start-Sleep -Milliseconds 100 }
    }
    if ($null -eq $checking) { throw 'Owned Excel did not become ready' }
    $excel = $checking.GetOwnedApplication()
    $excel.DisplayAlerts = $false
    $excel.Visible = $false
    $excel.Interactive = $false
    $excel.AutomationSecurity = 3
    $excel.EnableEvents = $false
    $excel.AutoRecover.Enabled = $false
    $excel.Calculation = -4135
    if ($Action -eq 'create') {
        if (Test-Path -LiteralPath $WorkbookPath) { throw 'Fixture already exists' }
        $workbook = $excel.Workbooks.Add()
        $sheet = $workbook.Worksheets.Item(1)
        $sheet.Name = 'Shared Fixture'
        $sheet.Cells.Item(1, 1).Value2 = 'shared data'
        $sheet.Cells.Item(2, 1).Value2 = 41.0
        $sheet.Cells.Item(2, 2).Formula = '=A2+1'
        $missing = [Type]::Missing
        $excel8Format = 56
        $sharedAccess = 2
        $workbook.SaveAs($WorkbookPath, $excel8Format, $missing, $missing, $false, $false, $sharedAccess)
        $workbook.KeepChangeHistory = $true
        $sheet.Cells.Item(2, 1).Value2 = 42.0
        $sheet.Calculate()
        $workbook.Save()
        $workbook.Close($false)
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
        $workbook = $null
    }
    $workbook = $excel.Workbooks.Open($WorkbookPath, 0, $true)
    $sheet = $workbook.Worksheets.Item(1)
    $isShared = [bool]$workbook.MultiUserEditing
    $receipt = @{
        multi_user_editing = $isShared
        keep_change_history = if ($isShared) { [bool]$workbook.KeepChangeHistory } else { $null }
        change_history_days = if ($isShared) { [int]$workbook.ChangeHistoryDuration } else { $null }
        sheet_name = [string]$sheet.Name
        a1_value = $sheet.Cells.Item(1, 1).Value2
        a2_value = $sheet.Cells.Item(2, 1).Value2
        b2_formula = [string]$sheet.Cells.Item(2, 2).Formula
        b2_value = $sheet.Cells.Item(2, 2).Value2
    }
    [IO.File]::WriteAllText($ReceiptPath, ($receipt | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
} finally {
    try {
        if ($null -ne $workbook) {
            $workbook.Close($false)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
        }
        if ($null -ne $checking) { $checking.Dispose() }
        if ($null -ne $excel) {
            $excel.Quit()
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
        }
    } finally {
        $owned.Dispose()
        $desktop.Dispose()
    }
}
