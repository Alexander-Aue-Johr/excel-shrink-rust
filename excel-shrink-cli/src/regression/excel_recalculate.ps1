param(
    [Parameter(Mandatory=$true)][string]$InputPath,
    [Parameter(Mandatory=$true)][string]$OutputPath,
    [Parameter(Mandatory=$true)][string]$ProgressPath
)

$ErrorActionPreference = 'Stop'

function Write-RecalcProgress([string]$message) {
    Add-Content -LiteralPath $ProgressPath -Encoding UTF8 `
        -Value "$(Get-Date -Format o) recalc $message"
}

Set-Content -LiteralPath $ProgressPath -Encoding UTF8 `
    -Value "$(Get-Date -Format o) recalc start"

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class ExcelShrinkRecalcWin32 {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
"@

$excel = $null
$workbook = $null
$settingsBook = $null

$stage = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    ("es-" + [guid]::NewGuid().ToString('N').Substring(0, 12))

New-Item -ItemType Directory -Path $stage | Out-Null
& (Join-Path $PSScriptRoot 'archive_processes.ps1') -StagePath $stage
Add-Type -Path (Join-Path $PSScriptRoot 'CaptureProcesses.cs')
$ownedProcesses = New-Object CaptureProcesses

try {
    $localInput = Join-Path $stage 'input.xlsx'
    $localOutput = Join-Path $stage 'output.xlsx'

    Write-RecalcProgress 'staging input'
    Copy-Item -LiteralPath $InputPath -Destination $localInput

    Write-RecalcProgress 'starting COM application'
    $excel = New-Object -ComObject Excel.Application

    $excelProcessId = [uint32]0
    [void][ExcelShrinkRecalcWin32]::GetWindowThreadProcessId(
        [IntPtr]$excel.Hwnd,
        [ref]$excelProcessId
    )
    $ownedProcesses.Add((Get-Process -Id $excelProcessId))
    Write-RecalcProgress "excel pid=$excelProcessId"

    # Keep Excel visible during regression recalculation.
    # Recalculation can legitimately take a long time, and a visible
    # application makes genuine Excel hangs distinguishable from slow work.
    $excel.Visible = $true

    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3
    $excel.EnableEvents = $false

    $settingsBook = $excel.Workbooks.Add()
    $excel.Calculation = -4135

    Write-RecalcProgress 'opening workbook'
    $workbook = $excel.Workbooks.Open($localInput, 0, $true)

    $workbook.ForceFullCalculation = $true

    # Blocking COM call. Rust owns timeout and wall-clock diagnostics.
    Write-RecalcProgress 'CalculateFullRebuild start'
    $excel.CalculateFullRebuild()
    Write-RecalcProgress 'CalculateFullRebuild finished'

    Write-RecalcProgress 'SaveAs start'
    $workbook.SaveAs($localOutput, 51)
    Write-RecalcProgress 'SaveAs finished'

    Write-RecalcProgress 'closing workbook'
    $workbook.Close($false)
    [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
    $workbook = $null

    Write-RecalcProgress 'copying recalculated output'
    Copy-Item -LiteralPath $localOutput -Destination $OutputPath -Force
}
finally {
    if ($null -ne $settingsBook) {
        try {
            $settingsBook.Close($false)
        }
        catch {
            Write-Warning $_.Exception.Message
        }

        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($settingsBook)
        $settingsBook = $null
    }

    if ($null -ne $workbook) {
        try {
            $workbook.Close($false)
        }
        catch {
            Write-Warning "Workbook close failed: $($_.Exception.Message)"
        }

        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
        $workbook = $null
    }

    if ($null -ne $excel) {
        Write-RecalcProgress 'quitting Excel'

        try {
            $excel.Quit()
        }
        finally {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
            $excel = $null
        }
    }

    $ownedProcesses.Dispose()
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}

Write-RecalcProgress 'finished'
