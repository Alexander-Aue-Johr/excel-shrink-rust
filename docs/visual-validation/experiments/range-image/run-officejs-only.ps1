param(
    [string]$CorpusRoot,
    [ValidateRange(1, 64)]
    [int]$OfficeJsBatchSize = 8,

    [ValidateSet('A', 'B', 'C', 'D', 'E', 'F')]
    [string]$Strategy = 'A',

    [ValidateSet('rust', 'dotnet')]
    [string]$Host = 'dotnet',

    # Source Context patch: forward manual task-pane start mode.
    [switch]$WaitForStart,

    # Source Context patch: forward Office.js diagnostics.
    [switch]$ExtendedErrorLogging,

    [switch]$UseOfficeDebugJs
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
    -WaitForStart:$WaitForStart `
    -ExtendedErrorLogging:$ExtendedErrorLogging `
    -UseOfficeDebugJs:$UseOfficeDebugJs `
    -NonInteractive
