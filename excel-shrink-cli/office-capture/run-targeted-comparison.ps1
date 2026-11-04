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
    [switch]$CheckPerformance,
    [switch]$SerialCaptures,
    [switch]$PrepareArtifactsOnly,
    [ValidateRange(1, 86400)][int]$OperationTimeoutSeconds = 120,
    [ValidateRange(1, 86400)][int]$CaptureIdleTimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$captureIdleTimedOut = $false
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = 'regression targeted-compare --original "{0}" --default "{1}" --output "{2}" --observatory-group-dir "{3}" --host-script "{4}"' -f `
        $Original, $Default, $Output, $ObservatoryGroup, (Join-Path $PSScriptRoot 'targeted-comparison-host.ps1')
    $arguments += ' --operation-timeout-seconds ' + $OperationTimeoutSeconds
    if ($SkipCheckPerformance -and $CheckPerformance) { throw 'Choose CheckPerformance or SkipCheckPerformance.' }
    if ($CheckPerformance) { $arguments += ' --check-performance' }
    if ($SerialCaptures) { $arguments += ' --serial-captures' }
    if ($PrepareArtifactsOnly) { $arguments += ' --prepare-artifacts-only' }
    $idle = [Diagnostics.Stopwatch]::StartNew()
    $lastCaptureSequence = 0L
    $process = $desktop.Start($Binary, $arguments, $owned)
    while (-not $process.WaitForExit(500)) {
        $telemetryPath = Join-Path $Output 'telemetry.json'
        if (Test-Path -LiteralPath $telemetryPath) {
            # A transient read or ordinary heartbeat must never extend the deadline.
            try {
                $progress = Get-Content -LiteralPath $telemetryPath -Raw | ConvertFrom-Json
                $sequence = [long]$progress.capture_success_sequence
                if ($sequence -gt $lastCaptureSequence) {
                    $lastCaptureSequence = $sequence
                    $idle.Restart()
                }
            } catch [System.IO.IOException] { }
              catch [System.ArgumentException] { }
        }
        if ($idle.Elapsed.TotalSeconds -ge $CaptureIdleTimeoutSeconds) {
            $captureIdleTimedOut = $true
            throw "No successful capture for $CaptureIdleTimeoutSeconds seconds; stopping this attempt."
        }
    }
    if ($process.ExitCode -ne 0) { throw "Targeted comparison failed; inspect $Output/failure.json." }
} finally {
    try { $owned.Dispose() }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        $desktop.Dispose()
        if ($captureIdleTimedOut) {
            # The CLI was terminated before it could publish a failure. Keep the
            # durable error and live mirror consistent with the launcher result.
            try {
                $message = "No successful capture for $CaptureIdleTimeoutSeconds seconds; stopping this attempt."
                [void][IO.Directory]::CreateDirectory($Output)
                $failure = @{status='failed';error=$message;kind='capture_idle_timeout';timeout_seconds=$CaptureIdleTimeoutSeconds}
                [IO.File]::WriteAllText((Join-Path $Output 'failure.json'),($failure|ConvertTo-Json))
                $local = Join-Path $Output 'telemetry.json'
                if (Test-Path -LiteralPath $local) {
                    $state = Get-Content -LiteralPath $local -Raw | ConvertFrom-Json
                    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    $state.status = 'failed'
                    $state.error = $message
                    $state.finished_at = $now
                    $state.updated_at = $now
                    foreach ($lane in $state.lanes) {
                        if ($lane.pid) {
                            $alive = $null -ne (Get-Process -Id $lane.pid -ErrorAction SilentlyContinue)
                            $lane.process_alive = $alive
                            $lane.lifecycle = if ($alive) { 'failed' } else { 'closed' }
                        }
                        foreach ($step in $lane.steps.PSObject.Properties) {
                            if ($step.Value -eq 'running') { $step.Value = 'failed' }
                        }
                    }
                    $json = $state | ConvertTo-Json -Depth 20
                    [IO.File]::WriteAllText($local,$json)
                    [IO.File]::WriteAllText((Join-Path $ObservatoryGroup 'targeted-comparison.json'),$json)
                }
            } catch { Write-Warning "Could not publish timeout evidence: $($_.Exception.Message)" }
        }
    }
}
