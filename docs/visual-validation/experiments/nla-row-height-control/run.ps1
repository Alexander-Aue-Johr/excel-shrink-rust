<#
.SYNOPSIS
Measures workbook-copy heights on an owned private desktop without taking user input.
#>
param(
    [Parameter(Mandatory=$true)][string]$Python,
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Original,
    [Parameter(Mandatory=$true)][string]$Output,
    [string]$Default,
    [string]$Sheet = '2007 NLA QA Data',
    [ValidateRange(1, 1048576)][int]$StartRow = 6,
    [ValidateRange(1, 1048576)][int]$EndRow = 179
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$outputPath = [IO.Path]::GetFullPath($Output)
$temporaryRoot = [IO.Path]::GetDirectoryName($outputPath)
[void][IO.Directory]::CreateDirectory($temporaryRoot)
$env:TEMP = $temporaryRoot
$env:TMP = $temporaryRoot
$env:EXCEL_SHRINK_LOG_DIR = Join-Path $temporaryRoot 'logs'
Import-Module (Join-Path $repo 'tools/corpus-launcher/Core.psm1') -Force
Add-Type -Path @(
    (Join-Path $repo 'excel-shrink-cli/office-capture/CaptureDesktop.cs'),
    (Join-Path $repo 'excel-shrink-cli/office-capture/CaptureProcesses.cs')
)
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = @(
        (Join-Path $PSScriptRoot 'same_package.py'), '--repository', $repo,
        '--binary', ([IO.Path]::GetFullPath($Binary)),
        '--original', ([IO.Path]::GetFullPath($Original)), '--output', $outputPath,
        '--sheet', $Sheet, '--start-row', [string]$StartRow, '--end-row', [string]$EndRow
    )
    if ($Default) { $arguments += @('--default', ([IO.Path]::GetFullPath($Default))) }
    $commandLine = ($arguments | ForEach-Object { ConvertTo-WindowsArgument $_ }) -join ' '
    $process = $desktop.Start([IO.Path]::GetFullPath($Python), $commandLine, $owned)
    if (-not $process.WaitForExit(600000)) { throw 'Same-package height probe exceeded ten minutes.' }
    if ($process.ExitCode -ne 0) { throw "Height probe failed. Inspect $outputPath/failure.txt and host.stderr.log." }
} finally {
    $owned.Dispose()
    $desktop.Dispose()
}
