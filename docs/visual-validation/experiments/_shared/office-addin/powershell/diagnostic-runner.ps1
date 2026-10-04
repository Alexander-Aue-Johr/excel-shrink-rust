# Shared orchestration for diagnostic experiments only; production capture is unchanged.
function Invoke-RenderDiagnostic {
    param(
        [Parameter(Mandatory=$true)][string]$RepoRoot,
        [Parameter(Mandatory=$true)][string]$SharedOfficeRunner,
        [Parameter(Mandatory=$true)][string]$Experiment,
        [Parameter(Mandatory=$true)][string[]]$SourcePaths,
        [Parameter(Mandatory=$true)][string]$LatestOutput,
        [hashtable]$Options = @{},
        [switch]$WaitForStart,
        [switch]$UseOfficeDebugJs,
        [switch]$BreakOnError
    )
    $ErrorActionPreference = 'Stop'
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $runRoot = New-VVStep $Experiment
    $Options.run_root = $runRoot
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $LatestOutput = Join-Path $env:VV_REPORT_DIR ('data\' + [IO.Path]::GetFileName($LatestOutput))
    $result = Join-Path $runRoot 'result.json'
    $artifacts = Join-Path $runRoot 'artifacts'
    $fileList = Join-Path $runRoot 'files.txt'
    $config = Join-Path $runRoot 'diagnostic-config.json'
    $Options.break_on_error = [bool]$BreakOnError
    $Options.probe_timeout_ms = 240000
    $Options.run_archive = $runId
    $Options.source_files = @($SourcePaths | ForEach-Object {
        if (-not (Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Workbook is missing: $_" }
        [ordered]@{ name = [IO.Path]::GetFileName($_); sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $Options.windows_version = [Environment]::OSVersion.VersionString
    $inputDir=Join-Path $runRoot 'inputs'; New-Item -ItemType Directory -Path $inputDir -Force | Out-Null
    foreach($source in $SourcePaths) {
        if ([IO.Path]::GetFileName($source) -like 'vv-*.xlsx') {Copy-Item -LiteralPath $source -Destination $inputDir -Force}
    }
    [IO.File]::WriteAllText($fileList, ($SourcePaths -join "`n") + "`n", $utf8)
    [IO.File]::WriteAllText($config, ($Options | ConvertTo-Json -Depth 30) + "`n", $utf8)
    $runnerArgs = @{
        FileListPath = $fileList; OutputPath = $result; ImageOutputDir = $artifacts
        Experiment = $Experiment; Strategy = 'A'; HostKind = 'dotnet'; ExcelLaunchMode = 'excel-x'
        BatchSize = 1; AttemptsPerWorkbook = 1; RetryDelaySeconds = 0
        StartupTimeoutSeconds = 180; InactivityTimeoutSeconds = 600; AbsoluteWorkbookTimeoutSeconds = 3600
        ExtendedErrorLogging = $true; DiagnosticConfigPath = $config
        WaitForStart = [bool]$WaitForStart; UseOfficeDebugJs = [bool]$UseOfficeDebugJs
        TemporaryDeveloperRegistration = $true
    }
    Write-Host "Diagnostic archive: $runRoot"
    try {
        & $SharedOfficeRunner @runnerArgs
        if (-not $?) { throw 'Diagnostic runner failed.' }
        if (-not (Test-Path -LiteralPath $result -PathType Leaf)) { throw "No final result: $result" }
        $data = [IO.File]::ReadAllText($result, [Text.Encoding]::UTF8) | ConvertFrom-Json
        $csvRows = @()
        foreach ($property in $data.workbooks.PSObject.Properties) {
            $workbook = $property.Value
            Write-Host ("Diagnostic status for {0}: {1}" -f $property.Name, $workbook.status)
            if ($workbook.status -ne 'complete') { Write-Warning 'Some diagnostic questions remain inconclusive. Inspect the saved JSON, not just the process exit code.' }
            if ($null -ne $workbook.analysis.height_plateau) {
                Write-Host ($workbook.analysis.height_plateau | Select-Object conclusion, plateau_px, first_plateau_index, last_below_plateau_index | ConvertTo-Json -Compress)
            }
            if ($null -ne $workbook.analysis.unit_conversion) {
                Write-Host ('Unit conversion: ' + ($workbook.analysis.unit_conversion | ConvertTo-Json -Depth 6 -Compress))
            }
            foreach ($probe in @($workbook.probes)) {
                $csvRows += [pscustomobject][ordered]@{
                    workbook = $property.Name; sequence = $probe.sequence; role = $probe.role
                    sheet = $probe.sheet; address = $probe.address; search_index = $probe.search_index
                    success = $probe.success; stage = $probe.stage
                    width_points = $probe.range_width_points; height_points = $probe.range_height_points
                    png_width = $probe.png_width; png_height = $probe.png_height
                    xml_column_width = $probe.xml_column_width; xml_row_height_points = $probe.xml_row_height_points
                    xml_sum_height_points = $probe.xml_sum_height_points
                    xml_sum_column_width_units = $probe.xml_sum_column_width_units
                    expected_width_px_96dpi = $probe.expected_width_px_96dpi
                    expected_height_px_96dpi = $probe.expected_height_px_96dpi
                    width_px_per_point_raw = $probe.width_px_per_point_raw
                    height_px_per_point_raw = $probe.height_px_per_point_raw
                    width_delta_from_96dpi_px = $probe.width_delta_from_96dpi_px
                    height_delta_from_96dpi_px = $probe.height_delta_from_96dpi_px
                    duration_ms = $probe.duration_ms
                    base64_chars = $probe.base64_chars; render_ms = $probe.render_ms
                    error_code = $(if ($null -ne $probe.error) { $probe.error.code } else { $null })
                    artifact = $probe.relative_png
                }
            }
        }
        $csv = Join-Path $runRoot 'probes.csv'
        $csvRows | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding UTF8
        New-Item -ItemType Directory -Path (Split-Path -Parent $LatestOutput) -Force | Out-Null
        Copy-Item -LiteralPath $result -Destination $LatestOutput -Force
        Copy-Item -LiteralPath $csv -Destination ([IO.Path]::ChangeExtension($LatestOutput, '.csv')) -Force
        Write-Host "Latest result: $LatestOutput"
        Write-Host "All probes CSV: $csv"
    }
    finally {
        Write-Host "Preserved diagnostics and raw marker PNGs: $artifacts"
        Write-Host "Run archive: $runRoot"
    }
}
