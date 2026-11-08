$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path (Join-Path $captureRoot 'ResidentProcesses.cs')
$owned = New-Object ResidentProcesses
$child = $null
try {
    $shell = (Get-Command powershell.exe).Source
    $child = $owned.Start($shell, '-NoProfile -Command "Start-Sleep -Seconds 60"')
    if ($child.HasExited) { throw 'Fixture exited before job termination.' }
    if (-not $owned.TerminateAndWait(5000)) { throw 'Owned job exit was not confirmed.' }
    if (-not $child.WaitForExit(1000)) { throw 'Child survived confirmed empty job.' }
    if (-not $owned.TerminateAndWait(1000)) { throw 'Repeated empty-job termination failed.' }
    Write-Host 'PASS owned job termination, empty-job confirmation and repeated cleanup'
} finally {
    $owned.Dispose()
    if ($null -ne $child) { $child.Dispose() }
}
