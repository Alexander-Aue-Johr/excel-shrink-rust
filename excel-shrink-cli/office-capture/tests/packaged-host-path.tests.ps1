$ErrorActionPreference = 'Stop'
$root = Join-Path $env:TEMP ('packaged-capture-host-' + [Guid]::NewGuid().ToString('N'))
$assets = Join-Path $root 'office-capture'
$null = New-Item -ItemType Directory -Path $assets
$module = Join-Path $assets 'CaptureSetup.psm1'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../CaptureSetup.psm1') -Destination $module
$previousHost = $env:EXCEL_SHRINK_CAPTURE_HOST
try {
    $env:EXCEL_SHRINK_CAPTURE_HOST = $null
    Import-Module $module -Force -DisableNameChecking
    $candidate = Join-Path $root 'deltasieve.exe'
    [IO.File]::WriteAllText($candidate, 'path-only fixture, never executed')
    if ((Get-CaptureHostExecutable '') -ne [IO.Path]::GetFullPath($candidate)) {
        throw 'Packaged DeltaSieve host was not selected.'
    }
    $explicit = Join-Path $root 'explicit.exe'
    [IO.File]::WriteAllText($explicit, 'path-only fixture, never executed')
    if ((Get-CaptureHostExecutable $explicit) -ne [IO.Path]::GetFullPath($explicit)) {
        throw 'Explicit native host did not take precedence.'
    }
    $env:EXCEL_SHRINK_CAPTURE_HOST = Join-Path $root 'missing.exe'
    $rejected = $false
    try { $null = Get-CaptureHostExecutable '' } catch { $rejected = $true }
    if (-not $rejected) { throw 'Missing explicit host silently fell back.' }
    Write-Output 'Packaged and explicit native host path checks passed.'
} finally {
    $env:EXCEL_SHRINK_CAPTURE_HOST = $previousHost
    Remove-Module CaptureSetup -ErrorAction SilentlyContinue
}
