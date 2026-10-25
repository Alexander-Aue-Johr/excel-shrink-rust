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
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try { return [ArchiveProcesses]::StopIfOwned([int]$owner.pid, [string]$owner.started) }
        catch {
            # Process teardown and security inspection can race. Retry the exact
            # identity, never weaken verification or silently drop the owner.
            if ($attempt -eq 3) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}

function Get-ArchiveStagePath([string]$stage) {
    if (-not [IO.Path]::IsPathRooted($stage)) { throw 'Archive stage must be an absolute path.' }
    $resolved = [IO.Path]::GetFullPath($stage).TrimEnd('\')
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
    $name = [IO.Path]::GetFileName($resolved)
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($resolved), $tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $name -notmatch '^(es-office-capture-[0-9a-f]{32}|es-open-[0-9a-f]{32}|es-[0-9a-f]{12})$') {
        throw "Refusing archive stage outside the known temporary staging directories: $resolved"
    }
    return $resolved
}

function Remove-ArchiveStage([string]$stage) {
    $resolved = Get-ArchiveStagePath $stage
    if (-not (Test-Path -LiteralPath $resolved)) { return }
    # Reject junctions/symlinks before a recursive delete. Enumerate one directory
    # at a time so validation itself never follows a reparse point.
    $pending = New-Object 'Collections.Generic.Stack[string]'
    $pending.Push($resolved)
    while ($pending.Count) {
        $entry = $pending.Pop()
        $attributes = [IO.File]::GetAttributes($entry)
        if ($attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Refusing archive stage containing a reparse point: $entry"
        }
        if ($attributes -band [IO.FileAttributes]::Directory) {
            foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($entry)) { $pending.Push($child) }
        }
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
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
        if ($owner.stage) {
            for ($attempt = 0; $attempt -lt 10; $attempt++) {
                try {
                    Remove-ArchiveStage ([string]$owner.stage)
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
    $stage = if ($StagePath) { Get-ArchiveStagePath $StagePath } else { $null }
    $record = @{ pid = $PID; started = [ArchiveProcesses]::CurrentStartedTicks(); stage = $stage }
    [IO.File]::AppendAllText($RegistryPath, (($record | ConvertTo-Json -Compress) + "`n"))
    return
}

Stop-ArchiveOwners ([IO.File]::ReadAllLines($RegistryPath))
