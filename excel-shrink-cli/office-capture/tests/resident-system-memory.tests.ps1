param([string]$SamplePath = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path (Join-Path $captureRoot 'ResidentProcesses.cs')

if ($SamplePath) {
    # This process runs inside a real 1 GiB Job. A process-local commit
    # allowance must not replace the system reserve measurement.
    $method = [ResidentProcesses].GetMethod('GetMemory', [Reflection.BindingFlags]'NonPublic,Static')
    $memory = $method.Invoke($null, @())
    $local = $memory.GetType().GetField('AvailablePage').GetValue($memory)
    @{
        system_commit_bytes = [ResidentProcesses]::AvailableCommitBytes
        process_commit_bytes = [UInt64]$local
        physical_bytes = [ResidentProcesses]::AvailablePhysicalBytes
    } | ConvertTo-Json | Set-Content -LiteralPath $SamplePath -Encoding UTF8
    exit 0
}

$sample = Join-Path ([IO.Path]::GetTempPath()) ('resident-memory-' + [Guid]::NewGuid().ToString('N') + '.json')
$job = [ResidentProcesses]::new(1024)
$child = $null
try {
    $shell = Join-Path $PSHOME 'powershell.exe'
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -SamplePath "' + $sample + '"'
    $child = $job.Start($shell, $arguments, $true)
    if (-not $child.WaitForExit(30000)) { throw 'Memory probe did not finish.' }
    if ($child.ExitCode -ne 0) { throw "Memory probe failed: $($child.ExitCode)" }
    $result = Get-Content -LiteralPath $sample -Raw | ConvertFrom-Json
    $system = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory
    $headroom = [UInt64]$system.CommitLimit - [UInt64]$system.CommittedBytes
    if ($headroom -lt 2GB) { throw 'This integration test requires at least 2 GiB system commit headroom.' }
    if ($result.system_commit_bytes -le 1GB) { throw 'System commit measurement incorrectly inherited the Job limit.' }
    # Independent performance counters are sampled later; allow normal activity.
    if ([Math]::Abs([double]$result.system_commit_bytes - $headroom) -gt 1GB) {
        throw 'Native system commit differs from the independent performance counters by more than 1 GiB.'
    }
    if ($result.physical_bytes -le 0) { throw 'Physical memory measurement is missing.' }
    $result | ConvertTo-Json
    'PASS: system commit headroom remains independent of a 1 GiB Job limit.'
} finally {
    if ($null -ne $child) {
        if (-not $child.HasExited) { [ResidentProcesses]::Stop($child) }
        $child.Dispose()
    }
    $job.Dispose()
    Remove-Item -LiteralPath $sample -Force -ErrorAction SilentlyContinue
}