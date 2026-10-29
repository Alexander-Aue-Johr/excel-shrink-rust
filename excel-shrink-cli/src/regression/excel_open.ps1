param(
    [Parameter(Mandatory=$true)][string]$InputPath,
    [Parameter(Mandatory=$true)][string]$MetricsPath,
    [Parameter(Mandatory=$true)][string]$ProgressPath,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ExcelProcessSlots.ps1')
function Progress([string]$message) {
    # Rust polls this file while Excel is running; neither handle may exclude readers.
    $bytes = [Text.Encoding]::UTF8.GetBytes("$(Get-Date -Format o) open $message" + [Environment]::NewLine)
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $stream = [IO.File]::Open($ProgressPath, [IO.FileMode]::Append, [IO.FileAccess]::Write, $share)
    try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
}
Add-Type -Path (Join-Path $PSScriptRoot 'CaptureProcesses.cs')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ExcelOpenWindow {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
'@

$stage = Join-Path $env:TEMP ('es-open-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
& (Join-Path $PSScriptRoot 'archive_processes.ps1') -StagePath $stage
$owned = New-Object CaptureProcesses
$excel = $null
$workbook = $null
$settings = $null
$excelSlot = $null
try {
    Progress 'staging input'
    $localInput = Join-Path $stage ('input' + [IO.Path]::GetExtension($InputPath))
    Copy-Item -LiteralPath $InputPath -Destination $localInput
    Progress 'starting Excel'
    $excelSlot = Enter-ExcelProcessSlot
    $excel = New-Object -ComObject Excel.Application
    $excelProcessId = [uint32]0
    [void][ExcelOpenWindow]::GetWindowThreadProcessId([IntPtr]$excel.Hwnd, [ref]$excelProcessId)
    $process = Get-Process -Id $excelProcessId
    $owned.Add($process)
    Progress "excel pid=$excelProcessId"
    $excel.Visible = $false
    $excel.Interactive = $false
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3
    $excel.EnableEvents = $false
    $settings = $excel.Workbooks.Add()
    $excel.Calculation = -4135

    Progress 'opening workbook'
    $timer = [Diagnostics.Stopwatch]::StartNew()
    # Empty password arguments make protected files fail instead of prompting.
    $missing = [Type]::Missing
    $workbook = $excel.Workbooks.Open(
        $localInput,  # Filename
        0,           # UpdateLinks
        $true,       # ReadOnly
        $missing,    # Format
        '',          # Password
        '',          # WriteResPassword
        $true        # IgnoreReadOnlyRecommended
    )
    $timer.Stop()
    $process.Refresh()
    $metrics = [ordered]@{
        open_seconds = $timer.Elapsed.TotalSeconds
        working_set_bytes = $process.WorkingSet64
        private_bytes = $process.PrivateMemorySize64
        calculation = 'manual'
    }
    if ($OutputPath) {
        Progress 'saving XLSX conversion'
        $metrics['macros_removed'] = [bool]$workbook.HasVBProject
        $localOutput = Join-Path $stage 'converted.xlsx'
        $workbook.CheckCompatibility = $false
        $workbook.SaveAs($localOutput, 51)
        Copy-Item -LiteralPath $localOutput -Destination $OutputPath -Force
    }
    $json = $metrics | ConvertTo-Json
    [IO.File]::WriteAllText($MetricsPath, $json, (New-Object Text.UTF8Encoding($false)))
    Progress 'finished'
} finally {
    # The cleanup job also handles an interrupted or blocked COM call.
    foreach ($book in @($workbook, $settings)) {
        if ($null -ne $book) {
            try { $book.Close($false) } catch {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($book)
        }
    }
    if ($null -ne $excel) {
        try { $excel.Quit() } catch {}
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
    }
    $owned.Dispose()
    Exit-ExcelProcessSlot $excelSlot
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
