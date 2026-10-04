param(
    [string]$CorpusRoot,

    [switch]$WaitForStart,

    [switch]$UseOfficeDebugJs,

    [switch]$BreakOnError,

    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\_shared\reporting.ps1')
$vvReportOwner = Start-VVReport -Start $PSScriptRoot -Experiment 'range-image-height-limit' -Arguments $PSBoundParameters
try {
$ErrorActionPreference = 'Stop'

$ExperimentsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$SharedOfficeRunner = Join-Path $ExperimentsRoot '_shared\office-addin\run.ps1'
$ResultsRoot = Join-Path $env:VV_REPORT_DIR 'data\height-limit'
$FileList = Join-Path $ResultsRoot 'files.txt'
$OutputPath = Join-Path $ResultsRoot 'officejs-dotnet-height-limit.json'
$ImageOutputDir = Join-Path $RepoRoot '.tmp\visual-validation\range-image-height-limit\images'
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

$WorkbookName = 'krankenhausverzeichnis-3500100227005.xlsx'
$TargetSheet = ' KHV_2022'
$FixedRange = 'A:FX'
$MaximumRow = 2791

if ([string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $CorpusRoot = Join-Path $RepoRoot 'corpus-local'
}
else {
    $CorpusRoot = [IO.Path]::GetFullPath($CorpusRoot)
}

if (-not (Test-Path -LiteralPath $CorpusRoot -PathType Container)) {
    throw "Corpus root does not exist: $CorpusRoot"
}

$preferredWorkbook = Join-Path `
    $CorpusRoot `
    ('original_destatis_xlsx_files\' + $WorkbookName)

if (Test-Path -LiteralPath $preferredWorkbook -PathType Leaf) {
    $WorkbookPath = [IO.Path]::GetFullPath($preferredWorkbook)
}
else {
    $matches = @(
        Get-ChildItem `
            -LiteralPath $CorpusRoot `
            -Recurse `
            -File `
            -Filter $WorkbookName `
            -ErrorAction Stop |
        Sort-Object FullName
    )

    if ($matches.Count -eq 0) {
        throw "Could not find fixed diagnostic workbook '$WorkbookName' under: $CorpusRoot"
    }
    if ($matches.Count -gt 1) {
        $paths = ($matches | ForEach-Object { $_.FullName }) -join "`n  "
        throw (
            "Found multiple copies of fixed diagnostic workbook '$WorkbookName'. " +
            "Expected exactly one or the original_destatis_xlsx_files copy.`n  $paths"
        )
    }

    $WorkbookPath = $matches[0].FullName
}

. (Join-Path $ExperimentsRoot '_shared\office-addin\powershell\diagnostic-runner.ps1')
Write-Host 'Range.getImage height-plateau diagnostic'
Write-Host ('  Workbook: ' + $WorkbookPath)
Write-Host '  Sheet: " KHV_2022"; main search A:FX, bottom row 1..2791.'
Write-Host '  Search uses actual PNG dimensions, not exception/no-exception.'
Invoke-RenderDiagnostic -RepoRoot $RepoRoot -SharedOfficeRunner $SharedOfficeRunner `
    -Experiment 'range-image-height-limit' -SourcePaths @($WorkbookPath) `
    -LatestOutput $OutputPath -Options @{ purpose = 'height-plateau-with-width-controls' } `
    -WaitForStart:$WaitForStart -UseOfficeDebugJs:$UseOfficeDebugJs -BreakOnError:$BreakOnError

} catch {
    $vvCaughtError = $_
    try { Write-VVFailure $vvCaughtError $PSCommandPath } catch { Write-Warning ('Could not persist error: ' + $_.Exception.Message) }
    throw $vvCaughtError
} finally {
    Stop-VVReport $vvReportOwner
}
