param(
    [string]$CorpusRoot,
    [switch]$SkipVsto,
    [switch]$SkipOfficeAddIn
)

$ErrorActionPreference = 'Stop'

$args = @{
    Implementation = 'run-all'
    NonInteractive = $true
}

if (-not [string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $args.CorpusRoot = $CorpusRoot
}

& (Join-Path $PSScriptRoot 'run.ps1') @args
