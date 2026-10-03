param(
    [string]$CorpusRoot,
    [ValidateRange(1, 64)]
    [int]$OfficeJsBatchSize = 8,

    [ValidateSet('A', 'B', 'C', 'D', 'E', 'F')]
    [string]$Strategy = 'A',

    [ValidateSet('rust', 'dotnet')]
    [string]$Host = 'dotnet',

    # Source Context patch: forward Office.js DevTools switch.
    [switch]$AutoOpenDevTools
)

$ErrorActionPreference = 'Stop'

$implementation = if ($Host -eq 'rust') {
    'officejs-rust'
}
else {
    'officejs-dotnet'
}

& (Join-Path $PSScriptRoot 'run.ps1') `
    -CorpusRoot $CorpusRoot `
    -Strategy $Strategy `
    -Implementation $implementation `
    -BatchSize $OfficeJsBatchSize `
    -AutoOpenDevTools:$AutoOpenDevTools `
    -NonInteractive
