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
Assert ([long]$started -eq [Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().Ticks) 'Existing journal birth times are not compatible.'
Assert (([ArchiveProcesses]::StopIfOwned($PID, '1')) -eq 'reused') 'A reused PID must survive.'
Assert (([ArchiveProcesses]::StopIfOwned([int]::MaxValue, $started)) -eq 'gone') 'An absent process must be harmless.'

# Reproduce the important permissions distinction with a real process: it can
# be queried but its DACL denies opening a new termination handle.
Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class CleanupProcessFixture {
    public static void DenyTerminate(IntPtr process) {
        Deny(process, "D:(D;;0x0001;;;WD)(A;;GA;;;WD)");
    }
    public static void DenyIdentity(IntPtr process) {
        Deny(process, "D:(D;;0x101001;;;WD)(A;;GA;;;WD)");
    }
    static void Deny(IntPtr process, string security) {
        IntPtr descriptor;
        uint size;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptor(security, 1, out descriptor, out size))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            if (!SetKernelObjectSecurity(process, 4, descriptor)) throw new Win32Exception(Marshal.GetLastWin32Error());
        } finally { LocalFree(descriptor); }
    }
    public static void Stop(IntPtr process) {
        if (!TerminateProcess(process, 1)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(string s, uint revision, out IntPtr descriptor, out uint size);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool SetKernelObjectSecurity(IntPtr handle, uint information, IntPtr descriptor);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr process, uint code);
}
'@
$fixture = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 120' -WindowStyle Hidden -PassThru
try {
    $fixtureHandle = $fixture.Handle
    $fixtureStarted = $fixture.StartTime.ToUniversalTime().Ticks
    [CleanupProcessFixture]::DenyTerminate($fixtureHandle)
    Assert (([ArchiveProcesses]::StopIfOwned($fixture.Id, '1')) -eq 'reused') 'Query-only verification requested unnecessary termination rights.'
    Assert (-not $fixture.HasExited) 'A recycled PID was terminated.'
    $denied = $false
    try { [ArchiveProcesses]::StopIfOwned($fixture.Id, [string]$fixtureStarted) | Out-Null }
    catch { $denied = $_.Exception.Message.Contains('acquire termination handle') -and $_.Exception.Message.Contains('Win32 5:') }
    Assert $denied 'A matching but inaccessible owner lost the native error and cleanup phase.'
    Assert (-not $fixture.HasExited) 'An inaccessible owner was incorrectly reported as stopped.'
} finally {
    [CleanupProcessFixture]::Stop($fixtureHandle)
    Assert ($fixture.WaitForExit(10000)) 'Fixture process did not terminate.'
    $fixture.Dispose()
}
$fixture = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 120' -WindowStyle Hidden -PassThru
try {
    $fixtureStarted = $fixture.StartTime.ToUniversalTime().Ticks
    Assert (([ArchiveProcesses]::StopIfOwned($fixture.Id, [string]$fixtureStarted)) -eq 'stopped') 'A confirmed owned process was not terminated.'
    Assert ($fixture.WaitForExit(10000)) 'Stopped fixture is still alive.'
    Assert (([ArchiveProcesses]::StopIfOwned($fixture.Id, [string]$fixtureStarted)) -eq 'gone') 'An exited process must be harmless.'
} finally { if (-not $fixture.HasExited) { $fixture.Kill() }; $fixture.Dispose() }
Write-Host 'PASS: query-only identity survives denied termination; owned, gone and reused processes are distinguished.'

# Reproduce an inaccessible replacement PID without relying on Windows to
# recycle a particular number during the test. Only an exact earlier identity
# with a confirmed-exit receipt is retired; a different birth stays unverified.
$receipt = Join-Path $env:TEMP ('es-owner-exit-test-' + [Guid]::NewGuid().ToString('N') + '.jsonl')
$confirmation = $receipt + '.confirmed'
$fixture = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 120' -WindowStyle Hidden -PassThru
try {
    $fixtureHandle = $fixture.Handle
    $fixtureStarted = $fixture.StartTime.ToUniversalTime().Ticks
    [CleanupProcessFixture]::DenyIdentity($fixtureHandle)
    [IO.File]::WriteAllText($receipt, ((@{pid=$fixture.Id;started='1'} | ConvertTo-Json -Compress) + "`n"))
    $retired = Read-ArchiveExitedOwners $receipt
    Stop-ArchiveOwners @((@{pid=$fixture.Id;started='1'} | ConvertTo-Json -Compress)) $confirmation $retired
    Assert ([IO.File]::Exists($confirmation)) 'A confirmed earlier exit attempted to reopen an inaccessible replacement PID.'
    Assert (-not $fixture.HasExited) 'Cleanup touched the inaccessible replacement process.'
    Remove-Item -LiteralPath $confirmation
    $blocked = $false
    try { Stop-ArchiveOwners @((@{pid=$fixture.Id;started=[string]$fixtureStarted} | ConvertTo-Json -Compress)) $confirmation $retired }
    catch { $blocked = $_.Exception.Message.Contains('Cannot confirm') }
    Assert $blocked 'An exit receipt for a different birth time authorized cleanup of a live unverified owner.'
    Assert (-not [IO.File]::Exists($confirmation)) 'Unverified owner published a cleanup confirmation.'
    [IO.File]::WriteAllText($receipt, '{"pid":1,"started":"not-a-time"}')
    $invalidReceipts = Read-ArchiveExitedOwners $receipt -WarningAction SilentlyContinue
    Assert ($invalidReceipts.Count -eq 0) 'A malformed confirmed-exit receipt was trusted.'
    $blocked = $false
    try { Stop-ArchiveOwners @((@{pid=$fixture.Id;started=[string]$fixtureStarted} | ConvertTo-Json -Compress)) $confirmation $invalidReceipts }
    catch { $blocked = $_.Exception.Message.Contains('Cannot confirm') }
    Assert $blocked 'An invalid exit receipt bypassed process identity verification.'
    Assert (-not [IO.File]::Exists($confirmation)) 'Invalid exit receipt authorized cleanup confirmation.'

    # Reproduce the retained journal: valid exit receipts followed by a
    # NUL-filled append. Preserve the valid evidence;
    # never interpret the damaged tail as another confirmed process exit.
    [IO.File]::WriteAllText($receipt, ((@{pid=$fixture.Id;started='1'} | ConvertTo-Json -Compress) + "`n" + ([string][char]0 * 43)))
    $retired = Read-ArchiveExitedOwners $receipt -WarningAction SilentlyContinue
    Assert ($retired.Count -eq 1) 'A torn exit append discarded valid earlier receipts or invented another identity.'
    Stop-ArchiveOwners @((@{pid=$fixture.Id;started='1'} | ConvertTo-Json -Compress)) $confirmation $retired
    Assert ([IO.File]::Exists($confirmation)) 'A torn advisory exit append prevented safe cleanup of a confirmed earlier exit.'
    Assert (-not $fixture.HasExited) 'A torn exit append authorized stopping a replacement process.'
    Write-Host 'PASS: valid exits survive torn receipts; invalid receipts confer no authority and require normal identity verification.'
} finally {
    [CleanupProcessFixture]::Stop($fixtureHandle)
    Assert ($fixture.WaitForExit(10000)) 'Inaccessible replacement fixture did not terminate.'
    $fixture.Dispose()
    Remove-Item -LiteralPath $receipt,$confirmation -ErrorAction SilentlyContinue
}

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
$goneStage = Join-Path $env:TEMP ('es-open-' + [Guid]::NewGuid().ToString('N'))
$junctionStage = Join-Path $env:TEMP ('es-open-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $registration = Join-Path $root 'owners.jsonl'
    Register-ArchiveOwner $registration @{pid=123;started='1'}
    $firstJournal = [IO.File]::ReadAllText($registration)
    [IO.File]::WriteAllText(($registration + '.stopping'), 'stopping')
    $blocked = $false
    try { Register-ArchiveOwner $registration @{pid=456;started='1'} } catch { $blocked=$true }
    Assert $blocked 'Cleanup did not reject a late launcher.'
    Assert ([IO.File]::ReadAllText($registration) -eq $firstJournal) 'Rejected launcher altered the ownership journal.'

    $generationJournal = Join-Path $root 'generation-owners.jsonl'
    $previousGeneration = $env:EXCEL_SHRINK_ARCHIVE_GENERATION
    try {
        [IO.File]::WriteAllText(($generationJournal + '.generation'), 'new-attempt')
        $env:EXCEL_SHRINK_ARCHIVE_GENERATION = 'expired-attempt'
        $blocked = $false
        try { Register-ArchiveOwner $generationJournal @{pid=456;started='1'} } catch { $blocked=$true }
        Assert $blocked 'A launcher from the previous attempt registered in a reused work directory.'
        Assert (-not [IO.File]::Exists($generationJournal)) 'An expired launcher polluted the new attempt journal.'
        $env:EXCEL_SHRINK_ARCHIVE_GENERATION = 'new-attempt'
        Register-ArchiveOwner $generationJournal @{pid=789;started='1'}
        Assert ([IO.File]::Exists($generationJournal)) 'The current attempt could not register.'
    } finally { $env:EXCEL_SHRINK_ARCHIVE_GENERATION = $previousGeneration }

    & {
        $raceJournal = Join-Path $root 'race-owners.jsonl'
        $script:registrationChecks = 0
        function Test-Path {
            param([string]$LiteralPath)
            $script:registrationChecks++
            if ($script:registrationChecks -eq 2) { [IO.File]::WriteAllText($LiteralPath, 'stopping') }
            Microsoft.PowerShell.Management\Test-Path -LiteralPath $LiteralPath
        }
        $launched = $false
        try {
            Register-ArchiveOwner $raceJournal @{pid=789;started='1'}
            $launched = $true
        } catch { }
        Assert (-not $launched) 'Registration racing the cleanup snapshot was allowed to start Excel.'
        Assert ([IO.File]::Exists($raceJournal)) 'Race fixture did not append before cleanup began.'
    }
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
    $confirmation = Join-Path $root 'owners-stopped'
    try { Stop-ArchiveOwners @((JournalLine 14 $unknownStage), (JournalLine 15)) $confirmation }
    catch { $blocked = $_.Exception.Message.Contains('Cannot confirm') }
    Assert $blocked 'An unverifiable active owner must block another worker.'
    Assert (-not (Test-Path -LiteralPath $confirmation)) 'An unverified owner published a stop confirmation.'
    Assert (Test-Path -LiteralPath $unknownStage) 'An unverified owner lost its files.'
    Assert (($script:visited -join ',') -eq '14,15') 'A denied process prevented cleanup of other launchers.'

    $script:visited = @()
    $script:states = @{ 15 = 'gone' }
    $blocked = $false
    try { Stop-ArchiveOwners @('not-json', (JournalLine 15)) $confirmation }
    catch { $blocked = $_.Exception.Message.Contains('Cannot confirm') }
    Assert $blocked 'A malformed ownership journal was silently discarded.'
    Assert (-not (Test-Path -LiteralPath $confirmation)) 'A malformed ownership journal authorized cleanup confirmation.'
    Assert (($script:visited -join ',') -eq '15') 'A malformed owner prevented closing other verifiable owners.'

    foreach ($invalid in @($env:TEMP, $root, (Join-Path $root 'es-123456789abc'), 'es-123456789abc')) {
        $refused = $false
        try { Remove-ArchiveStage $invalid } catch { $refused = $true }
        Assert $refused "Unsafe archive stage was accepted: $invalid"
    }
    Assert (Test-Path -LiteralPath $root) 'Stage validation removed an unrelated directory.'

    New-Item -ItemType Directory -Path $junctionStage | Out-Null
    $junction = Join-Path $junctionStage 'outside'
    New-Item -ItemType Junction -Path $junction -Target $root | Out-Null
    $refused = $false
    try { Remove-ArchiveStage $junctionStage } catch { $refused = $_.Exception.Message.Contains('reparse point') }
    Assert $refused 'Stage cleanup followed a junction.'
    Assert (Test-Path -LiteralPath $unknownStage) 'Stage cleanup deleted a junction target.'
    [IO.Directory]::Delete($junction)
    Remove-ArchiveStage $junctionStage

    # Once the launcher is confirmed gone, a locked/unsafe leftover directory is
    # only a warning. Its files remain, and another owned launcher is still handled.
    function Remove-ArchiveStage {
        Assert ([IO.File]::ReadAllText($confirmation) -eq 'owned-launchers-stopped') 'Optional file cleanup ran before owner-stop confirmation.'
        throw 'Deterministic sharing violation.'
    }
    function Start-Sleep {}
    $script:visited = @()
    $script:states = @{ 16 = 'gone'; 17 = 'gone' }
    Stop-ArchiveOwners @((JournalLine 16 $root), (JournalLine 17)) $confirmation
    Assert (($script:visited -join ',') -eq '16,17') 'Leftover stage files aborted cleanup.'
    Assert (Test-Path -LiteralPath $root) 'Failed stage cleanup removed files.'
    Write-Host 'PASS: absent/exiting and reused PIDs are safe; unverifiable owners block resume without skipping other cleanup.'
    Write-Host 'PASS: only known direct TEMP children are removable; leftover files cannot abort a safe resume.'
} finally {
    if (Test-Path -LiteralPath $junctionStage) {
        $junction = Join-Path $junctionStage 'outside'
        if (Test-Path -LiteralPath $junction) { [IO.Directory]::Delete($junction) }
        # This path was constructed above as a direct TEMP child with a GUID.
        $resolvedStage = Get-ArchiveStagePath $junctionStage
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $resolvedStage -Force
    }
    $resolvedRoot = [IO.Path]::GetFullPath($root)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        ([IO.Path]::GetFileName($resolvedRoot) -match '^es-archive-process-test-[0-9a-f]{32}$')) {
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
