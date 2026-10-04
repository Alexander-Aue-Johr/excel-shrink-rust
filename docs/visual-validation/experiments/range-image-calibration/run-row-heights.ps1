param(
    [switch]$WaitForStart,
    [switch]$UseOfficeDebugJs,
    [switch]$BreakOnError,
    [switch]$NonInteractive
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\_shared\reporting.ps1')
$vvReportOwner = Start-VVReport -Start $PSScriptRoot -Experiment 'row-heights' -Arguments $PSBoundParameters
try {
$ErrorActionPreference = 'Stop'
$ExperimentsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$SharedOfficeRunner = Join-Path $ExperimentsRoot '_shared\office-addin\run.ps1'
. (Join-Path $ExperimentsRoot '_shared\office-addin\powershell\diagnostic-runner.ps1')
$FixtureRoot = Join-Path $PSScriptRoot 'row-height-fixtures'
$manifestPath = Join-Path $FixtureRoot 'manifest.json'
$ResultsRoot = Join-Path $env:VV_REPORT_DIR 'data\row-heights'
$LatestOutput = Join-Path $ResultsRoot 'officejs-dotnet-row-heights.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw 'Row-height fixtures are missing. Apply patch_row_height_followup.py, or run generate-row-height-fixtures.py --apply from the repository root.'
}
$manifest = [IO.File]::ReadAllText($manifestPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
if ($manifest.schema -ne 'row-height-fixtures-v1') { throw 'Unexpected row-height fixture schema.' }
$paths = @()
foreach ($fixture in @($manifest.fixtures)) {
    $name = [string]$fixture.workbook
    if ([IO.Path]::GetFileName($name) -ne $name) { throw 'Invalid fixture filename.' }
    $path = Join-Path $FixtureRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing row-height fixture: $path" }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne [string]$fixture.sha256) { throw "Fixture changed: $path. Refusing mismatched XML metadata." }
    $paths += [IO.Path]::GetFullPath($path)
}
$options = @{ fixtures = @($manifest.fixtures); suite = 'row-heights'; fixture_schema = $manifest.schema }

function Export-RowHeightReview {
    param([string]$RunRoot, [string]$FixtureRoot, [string]$ResultsRoot)
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $resultPath = Join-Path $RunRoot 'result.json'
    if (Test-Path -LiteralPath $resultPath -PathType Leaf) {
        $data = [IO.File]::ReadAllText($resultPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
        $summary = [ordered]@{ experiment = 'row-heights'; workbooks = [ordered]@{} }
        $rows = New-Object 'System.Collections.Generic.List[object]'
        foreach ($property in $data.workbooks.PSObject.Properties) {
            $wb = $property.Value
            $summary.workbooks[$property.Name] = [ordered]@{
                status = $wb.status; summary = $wb.analysis.row_height_summary; fatal_error = $wb.analysis.fatal_error
            }
            foreach ($s in @($wb.analysis.row_height.snapshots)) {
                if ($null -eq $s) { continue }
                foreach ($r in @($s.rows)) {
                    $rows.Add([pscustomobject][ordered]@{
                        workbook = $property.Name; snapshot_id = $s.snapshot_id; study = $s.study
                        sheet = $s.sheet; phase = $s.phase; row = $r.row; case_id = $r.case_id
                        default_height_xml_pt = $s.default_row_height_xml_pt
                        sheet_custom_height_xml = $s.sheet_custom_height_xml
                        row_present_xml = $r.xml_row_present; row_ht_xml = $r.xml_ht
                        row_custom_height_xml = $r.xml_custom_height; row_style_xml = $r.xml_row_style
                        top_points = $r.top_points; height_points = $r.height_points
                        format_row_height_points = $r.format_row_height_points; hidden = $r.hidden
                        font_A = $r.font_A.name; font_A_size_pt = $r.font_A.size_pt
                        font_C = $r.font_C.name; font_C_size_pt = $r.font_C.size_pt
                        aggregate_before_points = $s.aggregate_before_row_reads.height_points
                        aggregate_after_points = $s.aggregate_after_row_reads.height_points
                        sum_row_height_points = $s.sum_individual_height_points
                        sum_minus_aggregate_points = $s.sum_minus_aggregate_after_points
                    })
                }
            }
        }
        $summaryPath = Join-Path $RunRoot 'row-height-summary.json'
        [IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 30) + "`n", $utf8)
        $csvPath = Join-Path $RunRoot 'row-geometry.csv'
        if ($rows.Count -gt 0) { $rows | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8 }
        Write-Host "Row-height summary: $summaryPath"
        Write-Host "Per-row geometry CSV: $csvPath"
    }
    New-Item -ItemType Directory -Path $ResultsRoot -Force | Out-Null
    $id = Split-Path -Leaf $RunRoot
    Write-Host ('Detailed row geometry: ' + $RunRoot)
}

Write-Host 'Row-height follow-up: automatic/default versus fixed heights, cell fonts, inherited fonts and per-row raster rounding.'
Write-Host 'The original calibration/marker fixtures and their latest results are unchanged.'
try {
    Invoke-RenderDiagnostic -RepoRoot $RepoRoot -SharedOfficeRunner $SharedOfficeRunner `
        -Experiment 'range-image-calibration' -SourcePaths $paths -LatestOutput $LatestOutput `
        -Options $options -WaitForStart:$WaitForStart -UseOfficeDebugJs:$UseOfficeDebugJs -BreakOnError:$BreakOnError
}
finally {
    if ($options.ContainsKey('run_archive')) {
        $runRoot = $options.run_root
        if (Test-Path -LiteralPath $runRoot -PathType Container) {
            try { Export-RowHeightReview -RunRoot $runRoot -FixtureRoot $FixtureRoot -ResultsRoot $ResultsRoot }
            catch { Write-Warning ('Could not export row-height summary: ' + $_.Exception.Message + '. Raw results remain at: ' + $runRoot) }
        }
    }
}

} catch {
    $vvCaughtError = $_
    try { Write-VVFailure $vvCaughtError $PSCommandPath } catch { Write-Warning ('Could not persist error: ' + $_.Exception.Message) }
    throw $vvCaughtError
} finally {
    Stop-VVReport $vvReportOwner
}
