param(
    [string]$RegistryPath = $env:EXCEL_SHRINK_ARCHIVE_OWNERS,
    [string]$StagePath,
    [switch]$Stop
)

$ErrorActionPreference = 'Stop'
if (-not $RegistryPath) { return }

if (-not $Stop) {
    $owner = Get-Process -Id $PID
    $record = @{ pid = $PID; started = $owner.StartTime.ToUniversalTime().Ticks.ToString(); stage = $StagePath }
    [IO.File]::AppendAllText($RegistryPath, (($record | ConvertTo-Json -Compress) + "`n"))
    return
}

# Verify process birth times: a stale journal must never stop a reused PID.
foreach ($line in [IO.File]::ReadAllLines($RegistryPath)) {
    if (-not $line.Trim()) { continue }
    $owner = $line | ConvertFrom-Json
    $process = Get-Process -Id ([int]$owner.pid) -ErrorAction SilentlyContinue
    if ($null -ne $process -and $process.StartTime.ToUniversalTime().Ticks.ToString() -eq $owner.started) {
        Stop-Process -Id $process.Id -Force -ErrorAction Stop
        [void]$process.WaitForExit(10000)
    }
    if ($owner.stage -and (Test-Path -LiteralPath $owner.stage)) {
        for ($attempt = 0; $attempt -lt 10; $attempt++) {
            try {
                Remove-Item -LiteralPath $owner.stage -Recurse -Force
                break
            } catch {
                if ($attempt -eq 9) { throw }
                Start-Sleep -Milliseconds 250
            }
        }
    }
}
