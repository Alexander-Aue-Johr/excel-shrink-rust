param(
    [Parameter(Mandatory = $true)][string]$SourceRepo,
    [Parameter(Mandatory = $true)][string]$BinaryPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "SheetScope executable is missing: $BinaryPath. Run the SheetScope build task first."
}

$assets = Join-Path $SourceRepo 'excel-shrink-cli\office-capture'
if (-not (Test-Path -LiteralPath $assets -PathType Container)) {
    throw "Office capture assets are missing: $assets"
}

# Windows Excel and its helper processes need a local Windows working directory.
$binaryHash = (Get-FileHash -LiteralPath $BinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
$deployment = Join-Path $env:LOCALAPPDATA "SheetScope\builds\$binaryHash"
$deployedBinary = Join-Path $deployment 'sheet-scope.exe'
$deployedAssets = Join-Path $deployment 'excel-shrink-cli\office-capture'

if (-not (Test-Path -LiteralPath $deployedBinary -PathType Leaf)) {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $deployedAssets))
    Copy-Item -LiteralPath $assets -Destination $deployedAssets -Recurse -Force
    Copy-Item -LiteralPath $BinaryPath -Destination $deployedBinary
}

Write-Host "Starting SheetScope from $deployment"
Push-Location $deployment
try {
    & $deployedBinary
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
