$ErrorActionPreference = 'Stop'
$registryScript = Join-Path $PSScriptRoot '../../src/regression/archive_processes.ps1'
. $registryScript -RegistryPath ''

function Assert([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
}

# Cleanup no longer obtains a nullable Process.StartTime property. A stable handle
# identifies even a process that exits while its birth time is being inspected.
function Get-Process { throw 'StartTime is unavailable in this deterministic race fixture.' }
Initialize-ArchiveProcesses
$started = [ArchiveProcesses]::CurrentStartedTicks()
Assert ([long]$started -gt 0) 'Current process birth time was not available.'
Assert (([ArchiveProcesses]::StopIfOwned($PID, '1')) -eq 'reused') 'A reused PID must survive.'
Assert (([ArchiveProcesses]::StopIfOwned([int]::MaxValue, $started)) -eq 'gone') 'An absent process must be harmless.'

$script:visited = @()
$script:states = @{}
function Stop-ArchiveProcess($owner) {
    $script:visited += [int]$owner.pid
    $state = $script:states[[int]$owner.pid]
    if ($state -eq 'unverifiable') { throw 'Access denied while inspecting active PID.' }
    return $state
}
function JournalLine([int]$id, [string]$stage = '') {
    return (@{ pid = $id; started = '1'; stage = $stage } | ConvertTo-Json -Compress)
}

$root = Join-Path $env:TEMP ('es-archive-process-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $goneStage = Join-Path $root 'exited-between-lookup-and-start-time'
    New-Item -ItemType Directory -Path $goneStage | Out-Null
    $script:states = @{ 11 = 'gone'; 12 = 'stopped' }
    Stop-ArchiveOwners @((JournalLine 11 $goneStage), (JournalLine 12))
    Assert (-not (Test-Path -LiteralPath $goneStage)) 'An exited launcher prevented stage cleanup.'
    Assert (($script:visited -join ',') -eq '11,12') 'Cleanup did not process both launchers.'

    $reusedStage = Join-Path $root 'reused-pid'
    New-Item -ItemType Directory -Path $reusedStage | Out-Null
    $script:states = @{ 13 = 'reused' }
    Stop-ArchiveOwners @((JournalLine 13 $reusedStage)) -WarningAction SilentlyContinue
    Assert (Test-Path -LiteralPath $reusedStage) 'A reused PID caused deletion of a journal stage.'

    $unknownStage = Join-Path $root 'unverified-owner'
    New-Item -ItemType Directory -Path $unknownStage | Out-Null
    $script:visited = @()
    $script:states = @{ 14 = 'unverifiable'; 15 = 'gone' }
    $blocked = $false
    try { Stop-ArchiveOwners @((JournalLine 14 $unknownStage), (JournalLine 15)) }
    catch { $blocked = $_.Exception.Message.Contains('Cannot confirm') }
    Assert $blocked 'An unverifiable active owner must block another worker.'
    Assert (Test-Path -LiteralPath $unknownStage) 'An unverified owner lost its files.'
    Assert (($script:visited -join ',') -eq '14,15') 'A denied process prevented cleanup of other launchers.'
    Write-Host 'PASS: absent/exiting and reused PIDs are safe; unverifiable owners block resume without skipping other cleanup.'
} finally {
    $resolvedRoot = [IO.Path]::GetFullPath($root)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        ([IO.Path]::GetFileName($resolvedRoot) -match '^es-archive-process-test-[0-9a-f]{32}$')) {
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
