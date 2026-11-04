<#
.SYNOPSIS
Polls compact Observatory diagnostics and retains only changed alerts and the latest snapshot.
.DESCRIPTION
This read-only watchdog never restarts or certifies workbooks. It records the exact
operation and process evidence for investigation. Run it alongside the corpus runner.
#>
param(
    [string]$Observatory = 'http://127.0.0.1:8771',
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [ValidateRange(5,300)][int]$IntervalSeconds = 15,
    [ValidateRange(5,86400)][int]$LongOperationSeconds = 120,
    [switch]$Once
)
$ErrorActionPreference = 'Stop'
$address = [Uri]$Observatory
if ($address.Scheme -ne 'http' -or $address.Host -notin @('127.0.0.1','localhost','::1')) {
    throw 'The watchdog requires a local Observatory address.'
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
[void][IO.Directory]::CreateDirectory($output)
$events = Join-Path $output 'alerts.jsonl'
$seen = @{}
$run = $null
$lastError = $null

function Write-WatchEvent($Event) {
    if ([IO.File]::Exists($events) -and (Get-Item -LiteralPath $events).Length -gt 16MB) {
        Move-Item -LiteralPath $events -Destination ($events + '.previous') -Force
    }
    $Event | ConvertTo-Json -Depth 12 -Compress | Add-Content -LiteralPath $events -Encoding UTF8
}

do {
    try {
        $snapshot = Invoke-RestMethod ($Observatory.TrimEnd('/') + "/api/summary?limit=50&stalled_after_seconds=$LongOperationSeconds") -TimeoutSec 20
        if ($snapshot.run_id -ne $run) { $seen.Clear(); $run = $snapshot.run_id }
        $pending = Join-Path $output ('latest-' + $PID + '.pending')
        $snapshot | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $pending -Encoding UTF8
        Move-Item -LiteralPath $pending -Destination (Join-Path $output 'latest.json') -Force
        $alerts = @($snapshot.comparisons | Where-Object { $_.attention -or $_.error })
        foreach ($alert in $alerts) {
            $key = "$($alert.comparison_id)/$($alert.current_operation.id)/$($alert.attention)/$($alert.error)"
            if (-not $seen.ContainsKey($key)) {
                Write-WatchEvent @{ at=[DateTime]::UtcNow.ToString('o'); run_id=$run; kind='comparison_attention'; evidence=$alert }
                $seen[$key] = $true
            }
        }
        if ($snapshot.runner_alive -eq $false -and $snapshot.running -eq $true) {
            $key = "runner-missing/$($snapshot.runner_pid)"
            if (-not $seen.ContainsKey($key)) {
                Write-WatchEvent @{ at=[DateTime]::UtcNow.ToString('o'); run_id=$run; kind='coordinator_missing'; pid=$snapshot.runner_pid }
                $seen[$key] = $true
            }
        }
        if ($Once) { $snapshot | ConvertTo-Json -Depth 16 }
        $lastError = $null
    } catch {
        $message = $_.Exception.Message
        if ($message -ne $lastError) {
            Write-WatchEvent @{ at=[DateTime]::UtcNow.ToString('o'); kind='observation_failed'; error=$message }
            $lastError = $message
        }
        if ($Once) { throw }
    }
    if (-not $Once) { Start-Sleep -Seconds $IntervalSeconds }
} while (-not $Once)
