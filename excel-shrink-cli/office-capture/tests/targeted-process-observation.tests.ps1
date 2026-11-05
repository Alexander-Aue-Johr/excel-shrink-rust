$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedProcessObservation.psm1') -Force
$current = [Diagnostics.Process]::GetCurrentProcess()
try {
    $started = ([DateTimeOffset]$current.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    if ((Get-TargetedProcessAlive -ProcessId $current.Id -StartedAt $started) -ne $true) {
        throw 'The exact live process identity was not recognized.'
    }
    if ((Get-TargetedProcessAlive -ProcessId $current.Id -StartedAt ($started + 1)) -ne $false) {
        throw 'A different birth time was treated as the recorded process.'
    }
    if ($null -ne (Get-TargetedProcessAlive -ProcessId $current.Id)) {
        throw 'Missing creation-time evidence was treated as a known identity.'
    }
    if ((Get-TargetedProcessAlive -ProcessId 2147483647 -StartedAt $started) -ne $false) {
        throw 'A missing process was not recognized.'
    }
    'Process identity observations passed; no processes were modified.'
} finally {
    $current.Dispose()
}
