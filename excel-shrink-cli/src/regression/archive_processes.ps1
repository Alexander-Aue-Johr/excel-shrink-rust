param(
    [string]$RegistryPath = $env:EXCEL_SHRINK_ARCHIVE_OWNERS,
    [string]$StagePath,
    [string]$StopConfirmationPath,
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

function Assert-ArchiveRegistration([string]$path) {
    $stopping = $path + '.stopping'
    if (Test-Path -LiteralPath $stopping) { throw 'Workbook cleanup has started; refusing a new Excel launcher.' }
    if ($env:EXCEL_SHRINK_ARCHIVE_GENERATION -and
        [IO.File]::ReadAllText($path + '.generation') -cne $env:EXCEL_SHRINK_ARCHIVE_GENERATION) {
        throw 'Workbook attempt changed; refusing an expired Excel launcher.'
    }
}

function Register-ArchiveOwner([string]$path, $record) {
    Assert-ArchiveRegistration $path
    [IO.File]::AppendAllText($path, (($record | ConvertTo-Json -Compress) + "`n"))
    # Pair with the parent's marker-before-stop ordering. A registration that
    # raced its journal snapshot cannot proceed to create Excel afterwards.
    Assert-ArchiveRegistration $path
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

function Get-ArchiveOwnerKey($owner) {
    $processId = 0
    $started = 0L
    if (-not [int]::TryParse([string]$owner.pid, [ref]$processId) -or $processId -le 0 -or
        -not [long]::TryParse([string]$owner.started, [ref]$started) -or $started -le 0) {
        throw 'Invalid process identity in the archive ownership journal.'
    }
    return ([string]$processId + '|' + [string]$started)
}

function Read-ArchiveExitedOwners([string]$path) {
    $exited = New-Object 'Collections.Generic.HashSet[string]'
    if ([IO.File]::Exists($path)) {
        foreach ($line in [IO.File]::ReadAllLines($path)) {
            if ($line.Trim()) { [void]$exited.Add((Get-ArchiveOwnerKey ($line | ConvertFrom-Json))) }
        }
    }
    return ,$exited
}

function Stop-ArchiveOwners($lines, [string]$confirmationPath = '', $exitedOwners = $null) {
    $failures = @()
    $stages = @()
    foreach ($line in $lines) {
        if (-not $line.Trim()) { continue }
        try {
            $owner = $line | ConvertFrom-Json
            # The Rust parent records this exact identity only after its retained
            # Child handle confirms exit. Do not reopen a potentially reused PID.
            $state = if ($null -ne $exitedOwners -and $exitedOwners.Contains((Get-ArchiveOwnerKey $owner))) {
                'gone'
            } else { Stop-ArchiveProcess $owner }
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
        if ($owner.stage) { $stages += [string]$owner.stage }
    }
    if ($failures.Count) {
        throw ('Cannot confirm that all owned Windows launchers have stopped: ' + ($failures -join '; '))
    }
    # The parent can now distinguish a blocked file deletion from an uncertain
    # live owner. Only the latter must prevent admitting another workbook.
    if ($confirmationPath) { [IO.File]::WriteAllText($confirmationPath, 'owned-launchers-stopped') }
    foreach ($stage in ($stages | Select-Object -Unique)) {
        for ($attempt = 0; $attempt -lt 10; $attempt++) {
            try {
                Remove-ArchiveStage $stage
                break
            } catch {
                if ($attempt -eq 9) {
                    # The owning launcher is gone. Leftover files cannot create
                    # a concurrent Excel session, so keep them and continue.
                    Write-Warning "Preserving archive stage ${stage}: $($_.Exception.Message)"
                } else { Start-Sleep -Milliseconds 250 }
            }
        }
    }
}

if (-not $RegistryPath) { return }

if (-not $Stop) {
    Initialize-ArchiveProcesses
    $stage = if ($StagePath) { Get-ArchiveStagePath $StagePath } else { $null }
    $record = @{ pid = $PID; started = [ArchiveProcesses]::CurrentStartedTicks(); stage = $stage }
    Register-ArchiveOwner $RegistryPath $record
    return
}

$exitedOwners = Read-ArchiveExitedOwners ($RegistryPath + '.exited')
Stop-ArchiveOwners ([IO.File]::ReadAllLines($RegistryPath)) $StopConfirmationPath $exitedOwners
