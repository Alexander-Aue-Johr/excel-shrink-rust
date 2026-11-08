<#
.SYNOPSIS
Runs an explicit reference plan on an owned private desktop.
.DESCRIPTION
The plan is independent of candidate selection. A bounded cleanup job owns the
runner and its Excel processes. This command never modifies corpus verdicts.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Original,
    [Parameter(Mandatory=$true)][string]$Default,
    [Parameter(Mandatory=$true)][string]$Plan,
    [Parameter(Mandatory=$true)][string]$Output,
    [ValidateRange(1, 600)][int]$OperationTimeoutSeconds = 120,
    [ValidateRange(1, 600)][int]$TotalTimeoutSeconds = 300
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$capture = Join-Path $PSScriptRoot '../excel-shrink-cli/office-capture'
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$originalPath = (Resolve-Path -LiteralPath $Original).Path
$defaultPath = (Resolve-Path -LiteralPath $Default).Path
$planPath = (Resolve-Path -LiteralPath $Plan).Path
$outputPath = [IO.Path]::GetFullPath($Output).TrimEnd('\')
if (Test-Path -LiteralPath $outputPath) { throw 'Reference output must be a new directory.' }
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$failure = $null
$exitCode = $null
$previousDesktop = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = 'regression reference-compare --original "{0}" --default "{1}" --plan "{2}" --output "{3}" --operation-timeout-seconds {4}' -f `
        $originalPath, $defaultPath, $planPath, $outputPath, $OperationTimeoutSeconds
    $process = $desktop.Start($binaryPath, $arguments, $owned)
    if (-not $process.WaitForExit($TotalTimeoutSeconds * 1000)) {
        throw "Reference comparison exceeded its $TotalTimeoutSeconds-second experiment budget."
    }
    $exitCode = $process.ExitCode
    if ($exitCode -ne 0) { throw "Reference comparison failed. Inspect $outputPath/reference.json." }
} catch {
    $failure = $_.Exception.Message
    throw
} finally {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previousDesktop
    try { $owned.Dispose() }
    catch {
        if (-not $failure) { $failure = $_.Exception.Message }
        throw
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        $desktop.Dispose()
        if (Test-Path -LiteralPath $outputPath) {
            $receipt = @{status=$(if($failure){'failed'}else{'completed'}); error=$failure;
                exit_code=$exitCode; total_timeout_seconds=$TotalTimeoutSeconds;
                finished_at=[DateTime]::UtcNow.ToString('o')}
            [IO.File]::WriteAllText((Join-Path $outputPath 'reference-launcher.json'),
                ($receipt | ConvertTo-Json -Depth 3), [Text.UTF8Encoding]::new($false))
        }
    }
}
