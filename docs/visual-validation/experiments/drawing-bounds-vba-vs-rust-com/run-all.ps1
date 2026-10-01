param(
    [string]$CorpusRoot,
    [switch]$SkipVsto,
    [switch]$SkipOfficeAddIn
)

$ErrorActionPreference = 'Stop'

$results = Join-Path $PSScriptRoot 'results'
$baseRunner = Join-Path $PSScriptRoot 'run-experiment.ps1'
$vstoRunner = Join-Path $PSScriptRoot 'vsto-addin\run-vsto.ps1'
$officeRunner = Join-Path $PSScriptRoot 'office-addin\run-office-addin.ps1'
$summarizer = Join-Path $PSScriptRoot 'summarize-results.ps1'
$fileList = Join-Path $results 'files.txt'

$baseArgs = @{
    DeferValidation = $true
}

if (-not [string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $baseArgs.CorpusRoot = $CorpusRoot
}

& $baseRunner @baseArgs

# Optional variants must never inherit a successful/partial result from an older run.
foreach ($stale in @(
    'vsto.json',
    'vsto-status.json',
    'office-addin.json',
    'office-addin.partial.json',
    'office-addin-status.json'
)) {
    Remove-Item `
        -LiteralPath (Join-Path $results $stale) `
        -Force `
        -ErrorAction SilentlyContinue
}

if (-not $SkipVsto) {
    & $vstoRunner `
        -FileListPath $fileList `
        -OutputPath (Join-Path $results 'vsto.json') `
        -SkipIfUnavailable
}

if (-not $SkipOfficeAddIn) {
    & $officeRunner `
        -FileListPath $fileList `
        -OutputPath (Join-Path $results 'office-addin.json') `
        -SkipIfUnavailable
}

& $summarizer `
    -ResultsDir $results `
    -FailOnMismatch
