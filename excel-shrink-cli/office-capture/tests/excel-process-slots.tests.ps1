$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../ExcelProcessSlots.ps1')
$root = Join-Path ([IO.Path]::GetTempPath()) ('sweep-slot-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$previousRoot = $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT
$previousSweep = $env:EXCEL_SHRINK_PREPARATION_SWEEP
$env:EXCEL_SHRINK_EXCEL_SLOT_ROOT = $root
$env:EXCEL_SHRINK_PREPARATION_SWEEP = '1'
$held = @()
$groups = @()
$waiter = $null
try {
    if (Get-Command Enter-ExcelProcessSlot, Enter-ExcelProcessSlots -ErrorAction SilentlyContinue) {
        throw 'A separate Excel instance admission gate still exists.'
    }
    for ($index = 0; $index -lt 40; $index++) {
        $held += [IO.File]::Open((Join-Path $root ('excel-{0:D2}.lock' -f $index)), 'OpenOrCreate', 'ReadWrite', 'None')
    }
    $controlPath = Join-Path $root 'control.json'
    [IO.File]::WriteAllText($controlPath, '{"excel_process_limit":1,"target_groups":0,"sweep_processes":2}')
    $groups += Enter-CaptureGroupSlot
    $groups += Enter-CaptureGroupSlot
    if ($groups.Count -ne 2) { throw 'Sweep concurrency was restricted by an unrelated limit.' }
    $helper = (Join-Path $PSScriptRoot '../ExcelProcessSlots.ps1').Replace("'", "''")
    $waiter = [PowerShell]::Create()
    [void]$waiter.AddScript(". '$helper'; `$slot = Enter-CaptureGroupSlot; `$slot.Dispose(); 'admitted'")
    $pending = $waiter.BeginInvoke()
    Start-Sleep -Milliseconds 300
    if ($pending.IsCompleted) { throw 'A third sweep bypassed the two-job setting.' }
    [IO.File]::WriteAllText($controlPath, '{"excel_process_limit":1,"target_groups":0,"sweep_processes":3}')
    if (-not $pending.AsyncWaitHandle.WaitOne(5000)) { throw 'Increasing the sweep slider did not admit the waiting job.' }
    $result = $waiter.EndInvoke($pending)
    if ($result -notcontains 'admitted') { throw 'Waiting sweep failed to acquire its job slot.' }
    Write-Output 'PASS: only sweep_processes gates sweeps; legacy Excel limits and comparison target do not; live increases admit waiting jobs.'
} finally {
    if ($null -ne $waiter) { $waiter.Stop(); $waiter.Dispose() }
    foreach ($slot in $groups) { $slot.Dispose() }
    foreach ($slot in $held) { $slot.Dispose() }
    $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT = $previousRoot
    $env:EXCEL_SHRINK_PREPARATION_SWEEP = $previousSweep
    $resolved = [IO.Path]::GetFullPath($root)
    if ($resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}