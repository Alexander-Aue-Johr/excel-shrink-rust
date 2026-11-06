param(
    [Parameter(Mandatory = $true)][string]$SourceRepo,
    [Parameter(Mandatory = $true)][string]$BinaryPath,
    [string]$ShrinkBinaryPath = '',
    [string]$Config = '',
    [switch]$Destatis,
    [switch]$NoOpen
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Destatis -and $Config) { throw 'Choose -Destatis or -Config.' }

if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "SheetScope executable is missing: $BinaryPath. Run the SheetScope build task first."
}

$assets = Join-Path $SourceRepo 'excel-shrink-cli\office-capture'
if (-not (Test-Path -LiteralPath $assets -PathType Container)) {
    throw "Office capture assets are missing: $assets"
}

# Windows Excel and its helper processes need a local Windows working directory.
$binaryHash = (Get-FileHash -LiteralPath $BinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not $ShrinkBinaryPath) {
    $ShrinkBinaryPath = Join-Path (Split-Path -Parent $BinaryPath) 'excel-shrink-cli.exe'
}
if (-not (Test-Path -LiteralPath $ShrinkBinaryPath -PathType Leaf)) {
    throw "Excel Shrink executable is missing: $ShrinkBinaryPath. Build both binaries first."
}
$shrinkHash = (Get-FileHash -LiteralPath $ShrinkBinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
$assetHashes = @(Get-ChildItem -LiteralPath $assets -Recurse -File | Sort-Object FullName | ForEach-Object {
    $_.FullName.Substring($assets.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
})
$recipe = [Text.Encoding]::UTF8.GetBytes((@($binaryHash, $shrinkHash) + $assetHashes) -join "`n")
$hasher = [Security.Cryptography.SHA256]::Create()
try {
    $buildKey = ([BitConverter]::ToString($hasher.ComputeHash($recipe))).Replace('-', '').Substring(0, 32).ToLowerInvariant()
} finally {
    $hasher.Dispose()
}
$deployment = Join-Path $env:LOCALAPPDATA "SheetScope\builds\$buildKey"
$deployedBinary = Join-Path $deployment 'sheet-scope.exe'
$deployedShrink = Join-Path $deployment 'excel-shrink-cli.exe'
$deployedAssets = Join-Path $deployment 'excel-shrink-cli\office-capture'

if (-not (Test-Path -LiteralPath $deployedBinary -PathType Leaf)) {
    [void][IO.Directory]::CreateDirectory($deployedAssets)
    Get-ChildItem -LiteralPath $assets -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $deployedAssets -Recurse -Force
    }
    Copy-Item -LiteralPath $ShrinkBinaryPath -Destination $deployedShrink
    Copy-Item -LiteralPath $BinaryPath -Destination $deployedBinary
}

$arguments = @('observatory')
if ($NoOpen) { $arguments += '--no-open' }
if ($Destatis) {
    $source = Join-Path $SourceRepo 'corpus-local\original_destatis_xlsx_files'
    if (-not (Test-Path -LiteralPath $source -PathType Container)) {
        throw "Destatis source folder is missing: $source"
    }
    $runRoot = Join-Path $env:LOCALAPPDATA ('SheetScope\runs\destatis-' + [Guid]::NewGuid().ToString('N'))
    $report = Join-Path $runRoot 'report'
    $arguments += @('--a', $source, '--excel-shrink', $deployedShrink, '--report', $report)
    Write-Host "Destatis report: $report"
    Write-Host 'Processed workbooks are temporary; the report is retained.'
}
if ($Config) {
    $Config = (Resolve-Path -LiteralPath $Config).Path
    $arguments += @('--config', $Config)
}

Write-Host "Starting SheetScope from $deployment"
Push-Location $deployment
try {
    & $deployedBinary @arguments
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
