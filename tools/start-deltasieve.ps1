[CmdletBinding()]
param(
    [ValidateSet('deltasieve', 'sheetshrink')][string]$Application = 'deltasieve',
    [string]$DataRoot = $env:DELTASIEVE_DATA_DIR,
    [string]$BinaryPath,
    [string]$Config,
    [ValidateRange(0, 65535)][int]$Port = 0,
    [switch]$NoOpen
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$packaged = Test-Path -LiteralPath (Join-Path $PSScriptRoot 'release-manifest.json')
$repo = if ($packaged) { $PSScriptRoot } else {
    [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
}
if (-not $DataRoot) {
    if ($packaged) {
        $DataRoot = Join-Path (Split-Path $repo -Parent) 'deltasieve-data'
    } elseif (Test-Path -LiteralPath 'V:\') {
        $DataRoot = 'V:\deltasieve-data'
    } else {
        throw 'Choose a working drive with -DataRoot or DELTASIEVE_DATA_DIR.'
    }
}
$DataRoot = [IO.Path]::GetFullPath($DataRoot)
if ($packaged -and ($DataRoot -ieq $repo -or
    $DataRoot.StartsWith($repo.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase))) {
    throw 'Choose a data directory outside the verified runtime package.'
}
$env:DELTASIEVE_DATA_DIR = $DataRoot
$env:TEMP = Join-Path $DataRoot 'temp'
$env:TMP = $env:TEMP
$env:EXCEL_SHRINK_LOG_DIR = Join-Path $DataRoot 'logs'
$env:EXCEL_SHRINK_VISUAL_LOG_DIR = $env:EXCEL_SHRINK_LOG_DIR
[void][IO.Directory]::CreateDirectory($env:TEMP)
if ($packaged) {
    $env:EXCEL_SHRINK_REPOSITORY_ROOT = $repo
    $env:EXCEL_SHRINK_CAPTURE_DIR = Join-Path $repo 'office-capture'
    if (-not $BinaryPath) { $BinaryPath = Join-Path $repo "$Application.exe" }
}

Push-Location $repo
try {
    if (-not $BinaryPath) {
        if (-not $env:CARGO_TARGET_DIR) {
            $env:CARGO_TARGET_DIR = Join-Path $DataRoot 'build'
        }
        $package = if ($Application -eq 'sheetshrink') { 'sheetshrink' } else { 'excel-shrink-cli' }
        & cargo build -p $package --bins -j 2
        if ($LASTEXITCODE -ne 0) { throw "$Application build failed." }
        $BinaryPath = Join-Path $env:CARGO_TARGET_DIR "debug\$Application.exe"
    }
    $BinaryPath = (Resolve-Path -LiteralPath $BinaryPath).Path
    if ($packaged) { $env:EXCEL_SHRINK_CAPTURE_HOST = $BinaryPath }
    $arguments = @('observatory', '--port', [string]$Port)
    if ($Config) {
        $arguments += @('--config', (Resolve-Path -LiteralPath $Config).Path)
    }
    if ($NoOpen) { $arguments += '--no-open' }
    & $BinaryPath @arguments
    if ($LASTEXITCODE -ne 0) { throw "$Application exited with code $LASTEXITCODE." }
} finally {
    Pop-Location
}
