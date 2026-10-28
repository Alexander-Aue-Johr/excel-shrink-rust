$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$captureRoot = Split-Path -Parent $PSScriptRoot
$repo = [IO.Path]::GetFullPath((Join-Path $captureRoot '../..'))
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst]
}, $false)) {
    if ($function.Name -in @('Get-SystemReserveBytes', 'Update-SystemReserve', 'Update-Resources', 'Wait-BatchRelease')) {
        . ([scriptblock]::Create($function.Extent.Text))
    }
}

# Exercise the real resource guard against fixed memory samples, without Excel,
# a capture host, process termination, or dependence on this machine's free RAM.
Add-Type @'
public static class ResidentProcesses {
    public static ulong AvailablePhysicalBytes { get; set; }
    public static ulong AvailableCommitBytes { get; set; }
    public static void PublishOnce(string source, string destination) {
        System.IO.File.Move(source, destination);
    }
}
'@
function Update-OtherGroupPause { }
function Read-OtherGroupPauseMillis { return $null }
function Add-SessionEvent { }
$script:checks = 0
function Assert($condition, [string]$message) {
    if (-not $condition) { throw $message }
    $script:checks++
}
function Assert-RamGuard([string]$message) {
    $blocked = $false
    try { Update-Resources }
    catch {
        if ($_.Exception.Message -notlike 'Resident RAM guard:*') { throw }
        $blocked = $true
    }
    Assert $blocked $message
}

$root = Join-Path $repo ('.tmp/resident-memory-reserve-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$liveControlPath = Join-Path $root 'control.json'
$hostProcess = $null
$liveOwner = $null
$batchLifetime = $null
$clients = @{}
$metricsClock = @{ ElapsedMilliseconds = 0 }
$metrics = @{
    status = 'running'
    system_reserve_mib = 4096; system_reserve_bytes = [UInt64]4GB
    minimum_available_bytes = [UInt64]::MaxValue; minimum_commit_bytes = [UInt64]::MaxValue
    clients = @{}
}
[ResidentProcesses]::AvailablePhysicalBytes = 768MB
[ResidentProcesses]::AvailableCommitBytes = 2GB
try {
    Assert ((Get-SystemReserveBytes $null) -eq 0) 'The default reserve is not disabled.'
    foreach ($value in @('0', '1', '512', '65536')) {
        Assert ((Get-SystemReserveBytes $value) -eq ([UInt64]$value * 1MB)) "Valid initial reserve rejected: $value"
    }
    foreach ($value in @('-1', '65537', '1.5', 'invalid', '')) {
        $rejected = $false
        try { Get-SystemReserveBytes $value | Out-Null } catch { $rejected = $true }
        Assert $rejected "Invalid initial reserve accepted: $value"
    }

    $script:systemReserveBytes = Get-SystemReserveBytes '4096'
    Assert-RamGuard 'A missing control file discarded the explicit initial reserve.'
    Assert ($metrics.system_reserve_mib -eq 4096) 'The explicit initial reserve was not reported.'

    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":0}', $utf8)
    Update-Resources
    Assert ($systemReserveBytes -eq 0 -and $metrics.system_reserve_mib -eq 0) 'Disabling the live guard did not take effect.'

    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":512}', $utf8)
    Update-Resources
    Assert ($systemReserveBytes -eq 512MB -and $metrics.system_reserve_mib -eq 512 -and
        $metrics.system_reserve_bytes -eq 512MB) 'Lowering the reserve did not immediately unblock the active guard.'

    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":1024}', $utf8)
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $writer = [IO.File]::Open($liveControlPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, $share)
    try { Assert-RamGuard 'Raising the live reserve while a writer holds the file did not immediately block.' }
    finally { $writer.Dispose() }
    Assert ($metrics.system_reserve_mib -eq 1024 -and $metrics.system_reserve_bytes -eq 1GB) 'The guard did not report the raised effective reserve.'

    [ResidentProcesses]::AvailablePhysicalBytes = 2GB
    [ResidentProcesses]::AvailableCommitBytes = 768MB
    Assert-RamGuard 'The live reserve stopped protecting commit headroom.'

    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":512}', $utf8)
    Update-Resources
    foreach ($invalid in @(
        '{', '{}', '{"system_reserve_mib":null}',
        '{"system_reserve_mib":-1}', '{"system_reserve_mib":65537}',
        '{"system_reserve_mib":1.5}', '{"system_reserve_mib":"1"}',
        '{"system_reserve_mib":true}', '{"system_reserve_mib":[1]}'
    )) {
        [IO.File]::WriteAllText($liveControlPath, $invalid, $utf8)
        Update-Resources
        Assert ($systemReserveBytes -eq 512MB -and $metrics.system_reserve_mib -eq 512) 'Missing or invalid live settings changed the last valid reserve.'
    }
    $oversized = '{"system_reserve_mib":65536,"padding":"' + ('x' * 17000) + '"}'
    [IO.File]::WriteAllText($liveControlPath, $oversized, $utf8)
    Update-Resources
    Assert ($systemReserveBytes -eq 512MB) 'An oversized control snapshot changed the reserve.'
    [IO.File]::Delete($liveControlPath)
    Update-Resources
    Assert ($systemReserveBytes -eq 512MB) 'A temporarily missing control snapshot reset the reserve.'

    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":1}', $utf8)
    [ResidentProcesses]::AvailablePhysicalBytes = 1MB
    [ResidentProcesses]::AvailableCommitBytes = 1MB
    Update-Resources
    Assert ($metrics.system_reserve_bytes -eq 1MB) 'The positive 1 MiB boundary was not applied.'
    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":65536}', $utf8)
    Assert-RamGuard 'The upper reserve boundary was ignored.'
    Assert ($metrics.system_reserve_mib -eq 65536) 'The upper reserve boundary was not reported.'

    # Retained clients waiting for peer groups must obey changes too. The first
    # iteration lowers the old 4 GiB reserve; the next receives a raised value.
    $script:systemReserveBytes = Get-SystemReserveBytes '4096'
    [ResidentProcesses]::AvailablePhysicalBytes = 768MB
    [ResidentProcesses]::AvailableCommitBytes = 768MB
    [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":512}', $utf8)
    $batchLifetime = @{
        ready_path = (Join-Path $root 'ready.json')
        release_path = (Join-Path $root 'release')
        timeout_seconds = 5
    }
    $batchOwner = @{ HasExited = $false }
    $batchReleased = $false
    $script:waitIterations = 0
    function Start-Sleep {
        param([int]$Milliseconds)
        $script:waitIterations++
        if ($waitIterations -gt 1) { throw 'Batch wait ignored the raised reserve.' }
        [IO.File]::WriteAllText($liveControlPath, '{"system_reserve_mib":2048}', $utf8)
    }
    Assert ((Wait-BatchRelease) -eq 'memory_guard') 'The batch guard did not apply a raised live reserve.'
    Assert ($waitIterations -eq 1) 'Lowering the reserve did not allow the first batch-wait iteration.'
    Assert ($metrics.system_reserve_mib -eq 2048 -and $metrics.system_reserve_bytes -eq 2GB) 'The batch guard reported an obsolete reserve.'
    Write-Host "PASS: $checks reserve checks; default off, live off/lower/raise, commit guard, shared writer, invalid/missing snapshots."
} finally {
    if ([IO.File]::Exists($liveControlPath)) { [IO.File]::Delete($liveControlPath) }
    $ready = Join-Path $root 'ready.json'
    if ([IO.File]::Exists($ready)) { [IO.File]::Delete($ready) }
    [IO.Directory]::Delete($root)
}
