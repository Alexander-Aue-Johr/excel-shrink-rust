param(
    [string]$CorpusRoot,
    [ValidateSet('all', 'units', 'markers', 'row-heights')][string]$Suite = 'all',
    [switch]$WaitForStart,
    [switch]$UseOfficeDebugJs,
    [switch]$BreakOnError,
    [switch]$NonInteractive
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\_shared\reporting.ps1')
$vvReportOwner = Start-VVReport -Start $PSScriptRoot -Experiment 'range-image-calibration' -Arguments $PSBoundParameters
try {
$ErrorActionPreference = 'Stop'
if ($Suite -eq 'row-heights') {
    & (Join-Path $PSScriptRoot 'run-row-heights.ps1') -WaitForStart:$WaitForStart `
        -UseOfficeDebugJs:$UseOfficeDebugJs -BreakOnError:$BreakOnError -NonInteractive:$NonInteractive
    return
}
$ExperimentsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$SharedOfficeRunner = Join-Path $ExperimentsRoot '_shared\office-addin\run.ps1'
. (Join-Path $ExperimentsRoot '_shared\office-addin\powershell\diagnostic-runner.ps1')
$FixtureRoot = Join-Path $PSScriptRoot 'fixtures'
$manifestPath = Join-Path $FixtureRoot 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw 'Diagnostic fixtures are missing. Reapply the patch or run generate-fixtures.py --apply from the repository root.'
}
$manifest = [IO.File]::ReadAllText($manifestPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
$fixtures = @($manifest.fixtures | Where-Object { $Suite -eq 'all' -or $_.kind -eq $Suite })
if ($fixtures.Count -eq 0) { throw "No fixtures selected for suite: $Suite" }
$paths = @()
foreach ($fixture in $fixtures) {
    $path = Join-Path $FixtureRoot ([string]$fixture.workbook)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Fixture is missing: $path" }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne [string]$fixture.sha256) { throw "Fixture changed: $path. Do not measure edited fixtures with stale XML metadata." }
    $paths += [IO.Path]::GetFullPath($path)
}
Write-Host 'Synthetic OOXML diagnostics: units, zoom, borders, rounding and narrow marker strips.'
Write-Host 'Each fixture is opened as a disposable copy. No hospital workbook is required for this experiment.'
Invoke-RenderDiagnostic -RepoRoot $RepoRoot -SharedOfficeRunner $SharedOfficeRunner `
    -Experiment 'range-image-calibration' -SourcePaths $paths `
    -LatestOutput (Join-Path $env:VV_REPORT_DIR 'data\officejs-dotnet-calibration.json') `
    -Options @{ fixtures = $fixtures; suite = $Suite; fixture_schema = $manifest.schema } `
    -WaitForStart:$WaitForStart -UseOfficeDebugJs:$UseOfficeDebugJs -BreakOnError:$BreakOnError

} catch {
    $vvCaughtError = $_
    try { Write-VVFailure $vvCaughtError $PSCommandPath } catch { Write-Warning ('Could not persist error: ' + $_.Exception.Message) }
    throw $vvCaughtError
} finally {
    Stop-VVReport $vvReportOwner
}
