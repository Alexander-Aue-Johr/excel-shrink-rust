$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../ExcelProcessSlots.ps1')
$root = Join-Path ([IO.Path]::GetTempPath()) ('excel-slot-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$previous = $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT
$env:EXCEL_SHRINK_EXCEL_SLOT_ROOT = $root
$held = @()
try {
    for ($index = 0; $index -lt 40; $index++) {
        $held += [IO.File]::Open((Join-Path $root ('excel-{0:D2}.lock' -f $index)), 'OpenOrCreate', 'ReadWrite', 'None')
    }
    [IO.File]::WriteAllText((Join-Path $root 'control.json'), '{"excel_process_limit":0,"target_groups":0,"sweep_processes":2}')
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $slots = @(Enter-ExcelProcessSlots 4)
    if ($slots.Count -ne 4 -or $clock.Elapsed.TotalSeconds -gt 2) { throw 'Unlimited admission waited for occupied Excel slots.' }
    foreach ($slot in $slots) { $slot.Dispose() }
    $env:EXCEL_SHRINK_PREPARATION_SWEEP = '1'
    $group = Enter-CaptureGroupSlot
    if ($null -eq $group) { throw 'Sweep admission did not use its own pool.' }
    $group.Dispose()
    foreach ($slot in $held) { $slot.Dispose() }; $held = @()
    [IO.File]::WriteAllText((Join-Path $root 'control.json'), '{"excel_process_limit":40}')
    $slots = @(Enter-ExcelProcessSlots 34)
    if ($slots.Count -ne 34) { throw 'Explicit admission above 32 slots failed.' }
    foreach ($slot in $slots) { $slot.Dispose() }
    Write-Output 'PASS: unlimited Excel admission, independent sweep pool, and explicit limits above 32.'
} finally {
    foreach ($slot in $held) { $slot.Dispose() }
    $env:EXCEL_SHRINK_EXCEL_SLOT_ROOT = $previous
    Remove-Item Env:EXCEL_SHRINK_PREPARATION_SWEEP -ErrorAction SilentlyContinue
    $resolved = [IO.Path]::GetFullPath($root)
    if ($resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
