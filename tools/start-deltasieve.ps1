[CmdletBinding()]
param(
    [string]$DataRoot = $env:DELTASIEVE_DATA_DIR,
    [string]$BinaryPath,
    [string]$Config,
    [ValidateRange(0, 65535)][int]$Port = 0,
    [switch]$NoOpen
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $DataRoot) {
    if (-not (Test-Path -LiteralPath 'V:\')) {
        throw 'Choose a working drive with -DataRoot or DELTASIEVE_DATA_DIR.'
    }
    $DataRoot = 'V:\deltasieve-data'
}
$DataRoot = [IO.Path]::GetFullPath($DataRoot)
$env:DELTASIEVE_DATA_DIR = $DataRoot
$env:TEMP = Join-Path $DataRoot 'temp'
$env:TMP = $env:TEMP
$env:EXCEL_SHRINK_LOG_DIR = Join-Path $DataRoot 'logs'
[void][IO.Directory]::CreateDirectory($env:TEMP)

Push-Location $repo
try {
    if (-not $BinaryPath) {
        if (-not $env:CARGO_TARGET_DIR) {
            $env:CARGO_TARGET_DIR = Join-Path $DataRoot 'build'
        }
        & cargo build -p excel-shrink-cli --bins -j 2
        if ($LASTEXITCODE -ne 0) { throw 'DeltaSieve build failed.' }
        $BinaryPath = Join-Path $env:CARGO_TARGET_DIR 'debug\deltasieve.exe'
    }
    $BinaryPath = (Resolve-Path -LiteralPath $BinaryPath).Path
    $arguments = @('observatory', '--port', [string]$Port)
    if ($Config) {
        $arguments += @('--config', (Resolve-Path -LiteralPath $Config).Path)
    }
    if ($NoOpen) { $arguments += '--no-open' }
    & $BinaryPath @arguments
    if ($LASTEXITCODE -ne 0) { throw "DeltaSieve exited with code $LASTEXITCODE." }
} finally {
    Pop-Location
}
