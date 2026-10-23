param(
    [string]$RegistryPath = $env:EXCEL_SHRINK_ARCHIVE_OWNERS,
    [string]$StagePath,
    [switch]$Stop
)

$ErrorActionPreference = 'Stop'

function Initialize-ArchiveProcesses {
    if (-not ('ArchiveProcesses' -as [type])) {
        Add-Type -Path (Join-Path $PSScriptRoot 'ArchiveProcesses.cs')
    }
}

function Stop-ArchiveProcess($owner) {
    Initialize-ArchiveProcesses
    return [ArchiveProcesses]::StopIfOwned([int]$owner.pid, [string]$owner.started)
}

function Stop-ArchiveOwners($lines) {
    $failures = @()
    foreach ($line in $lines) {
        if (-not $line.Trim()) { continue }
        try {
            $owner = $line | ConvertFrom-Json
            $state = Stop-ArchiveProcess $owner
            if ($state -eq 'reused') {
                Write-Warning "Archive launcher PID $($owner.pid) was reused; preserving its journal stage."
                continue
            }
        } catch {
            # Continue closing the other owned jobs, but never start another worker
            # while a journaled launcher could still be alive and unverifiable.
            $failures += $_.Exception.Message
            continue
        }
        if ($owner.stage -and (Test-Path -LiteralPath $owner.stage)) {
            for ($attempt = 0; $attempt -lt 10; $attempt++) {
                try {
                    Remove-Item -LiteralPath $owner.stage -Recurse -Force
                    break
                } catch {
                    if ($attempt -eq 9) {
                        # The owning launcher is gone. Leftover files cannot create
                        # a concurrent Excel session, so keep them and continue.
                        Write-Warning "Preserving archive stage $($owner.stage): $($_.Exception.Message)"
                    } else { Start-Sleep -Milliseconds 250 }
                }
            }
        }
    }
    if ($failures.Count) {
        throw ('Cannot confirm that all owned Windows launchers have stopped: ' + ($failures -join '; '))
    }
}

if (-not $RegistryPath) { return }

if (-not $Stop) {
    Initialize-ArchiveProcesses
    $record = @{ pid = $PID; started = [ArchiveProcesses]::CurrentStartedTicks(); stage = $StagePath }
    [IO.File]::AppendAllText($RegistryPath, (($record | ConvertTo-Json -Compress) + "`n"))
    return
}

Stop-ArchiveOwners ([IO.File]::ReadAllLines($RegistryPath))
