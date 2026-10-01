param(
    [string]$CorpusRoot,
    [ValidateRange(1, 64)]
    [int]$OfficeJsBatchSize = 8,
    [switch]$SkipVba,
    [switch]$SkipOfficeJs
)

$ErrorActionPreference = 'Stop'

if ($SkipVba -and $SkipOfficeJs) {
    throw 'Both VBA and Office.js are disabled.'
}

$runner = Join-Path $PSScriptRoot 'run.ps1'

if (-not $SkipVba) {
    & $runner `
        -CorpusRoot $CorpusRoot `
        -Strategy A `
        -Implementation vba `
        -BatchSize $OfficeJsBatchSize `
        -NonInteractive
}

if (-not $SkipOfficeJs) {
    & $runner `
        -CorpusRoot $CorpusRoot `
        -Strategy A `
        -Implementation officejs-dotnet `
        -BatchSize $OfficeJsBatchSize `
        -NonInteractive
}
