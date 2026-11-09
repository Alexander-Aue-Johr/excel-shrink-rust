$ErrorActionPreference = 'Stop'

function Invoke-WorkerPerformance {
    <# .SYNOPSIS
    Retain saved controls and confirmed optimization in one reusable owned Excel.
    The caller closes this workbook before the next request is admitted.
    #>
    param($Excel, $Workbook, $Process, $Request, [string]$Desktop)
    Import-Module (Join-Path $PSScriptRoot 'CheckPerformance.psm1') -DisableNameChecking
    $previousDesktop = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
    $previousVisible = $Excel.Visible
    $previousInteractive = $Excel.Interactive
    try {
        $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $Desktop
        $Excel.Visible = $true
        $Excel.Interactive = $true
        $Workbook.CheckCompatibility = $false
        $working = Join-Path $Request.directory 'performance-working.xlsx'
        $Workbook.SaveAs($working, 51, [Type]::Missing, [Type]::Missing, $false, $false, 3)
        Copy-Item -LiteralPath $working -Destination (Join-Path $Request.output 'original-saved.xlsx')
        $Workbook.Save()
        Copy-Item -LiteralPath $working -Destination (Join-Path $Request.output 'before-optimization.xlsx')
        $native = Invoke-WorkbookPerformanceOptimization @{
            excel=$Excel; workbook=$Workbook; process=$Process
            reset_pane=$true
            trace_path=(Join-Path $Request.directory 'check-performance.trace.log')
        } 120
        if ($native.status -notin @('optimization_confirmed', 'no_optimization_needed')) {
            throw 'Check Performance returned no confirmed outcome.'
        }
        $Workbook.Save()
        Copy-Item -LiteralPath $working -Destination (Join-Path $Request.output 'after-optimization.xlsx')
        return $native
    } finally {
        $Excel.Visible = $previousVisible
        $Excel.Interactive = $previousInteractive
        if ($null -eq $previousDesktop) {
            Remove-Item Env:\EXCEL_SHRINK_EXPECTED_DESKTOP -ErrorAction SilentlyContinue
        } else {
            $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previousDesktop
        }
    }
}

Export-ModuleMember -Function Invoke-WorkerPerformance
