<#
.SYNOPSIS
Runs the targeted comparison on an owned private desktop.
.DESCRIPTION
Both the CLI and its Excel hosts inherit this desktop. The expected-desktop
guard prevents visible Excel on the user's desktop. Closing the cleanup job
releases only this comparison's processes, including interrupted runs.
#>
param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Original,
    [Parameter(Mandatory=$true)][string]$Default,
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][string]$ObservatoryGroup,
    [switch]$SkipCheckPerformance,
    [switch]$SerialCaptures,
    [ValidateRange(1, 86400)][int]$OperationTimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = 'regression targeted-compare --original "{0}" --default "{1}" --output "{2}" --observatory-group-dir "{3}" --host-script "{4}"' -f `
        $Original, $Default, $Output, $ObservatoryGroup, (Join-Path $PSScriptRoot 'targeted-comparison-host.ps1')
    $arguments += ' --operation-timeout-seconds ' + $OperationTimeoutSeconds
    if ($SkipCheckPerformance) { $arguments += ' --skip-check-performance' }
    if ($SerialCaptures) { $arguments += ' --serial-captures' }
    $process = $desktop.Start($Binary, $arguments, $owned)
    while (-not $process.WaitForExit(500)) { }
    if ($process.ExitCode -ne 0) { throw "Targeted comparison failed; inspect $Output/failure.json." }
} finally {
    try { $owned.Dispose() }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        $desktop.Dispose()
    }
}
