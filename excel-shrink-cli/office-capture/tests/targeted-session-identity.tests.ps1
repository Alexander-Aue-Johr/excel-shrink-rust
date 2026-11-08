$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedSessionIdentity.psm1') -Force
$previous = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = 'group-desktop'
    if ((Get-TargetedSessionDesktop @{desktop_name='worker-desktop'}) -ne 'worker-desktop') {
        throw 'Worker desktop did not take precedence.'
    }
    if ((Get-TargetedSessionDesktop @{}) -ne 'group-desktop') { throw 'Legacy fallback changed.' }
    foreach ($entry in @(@{desktop_name='Default'}, @{desktop_name='bad"argument'}, @{})) {
        $env:EXCEL_SHRINK_EXPECTED_DESKTOP = ''
        $failed = $false
        try { Get-TargetedSessionDesktop $entry | Out-Null } catch { $failed = $true }
        if (-not $failed) { throw 'A missing or interactive desktop was accepted.' }
    }
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = 'Default'
    $failed = $false
    try { Get-TargetedSessionDesktop @{} | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw 'Interactive fallback was accepted.' }
} finally {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previous
}
Write-Output 'Targeted session desktop checks passed.'
