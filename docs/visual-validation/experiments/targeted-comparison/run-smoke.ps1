param(
    [Parameter(Mandatory=$true)][string]$Output,
    [switch]$FailCapture,
    [switch]$CheckPerformance,
    [switch]$FailOptimization,
    [switch]$CleanWorkbook,
    [switch]$RecalcCandidate,
    [switch]$SaveOverlap,
    [switch]$FailLateCapture,
    [switch]$Charts,
    [ValidateSet('constant','constant-string','styles','dummy-missing','dummy-inert','dummy-present','dummy-added')]
    [string]$ConditionalFormatting
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
$directory = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $directory) { throw 'Use a new smoke-test directory.' }
$null = New-Item -ItemType Directory -Path $directory
$fixtureArguments = @()
if ($CheckPerformance -or $FailOptimization -or $CleanWorkbook) { $fixtureArguments += '--check-performance' }
if ($CleanWorkbook) { $fixtureArguments += '--clean-workbook' }
if ($RecalcCandidate) { $fixtureArguments += '--recalc-candidate' }
if ($SaveOverlap) { $fixtureArguments += '--save-overlap' }
if ($Charts) { $fixtureArguments += '--charts' }
if ($ConditionalFormatting) { $fixtureArguments += @('--conditional-formatting', $ConditionalFormatting) }
& python (Join-Path $PSScriptRoot 'make_smoke_fixtures.py') (Join-Path $directory 'inputs') @fixtureArguments
if ($LASTEXITCODE -ne 0) { throw 'Fixture creation failed.' }
$sourceHashes = @{}
foreach ($name in @('original', 'default')) {
    $sourceHashes[$name] = (Get-FileHash (Join-Path $directory "inputs/$name.xlsx") -Algorithm SHA256).Hash
}
$group = Join-Path $directory 'control/groups/smoke'
$null = New-Item -ItemType Directory -Path $group -Force
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$previousDesktop = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
$hostScript = Join-Path $capture 'targeted-comparison-host.ps1'
if ($FailCapture -or $FailOptimization -or $FailLateCapture) {
    # Fault injection stays in a disposable host copy, after real Excel startup.
    $source = [IO.File]::ReadAllText($hostScript)
    if ($FailCapture) {
        $source = $source.Replace('default { Invoke-TargetedJob $command }', 'default { if ($command.kind -eq "capture") { throw "Injected capture failure" }; Invoke-TargetedJob $command }')
    }
    if ($FailLateCapture) {
        $source = $source.Replace('default { Invoke-TargetedJob $command }', 'default { if ($command.kind -eq "capture" -and $command.lane -eq "OR") { throw "Injected late capture failure" }; Invoke-TargetedJob $command }')
    }
    if ($FailOptimization) {
        $source = $source.Replace("'optimize' { Optimize-TargetedWorkbook `$command }", "'optimize' { @{status='invoked_completion_unverified'} }")
    }
    $module = (Join-Path $capture 'CheckPerformance.psm1').Replace("'", "''")
    $source = $source.Replace("(Join-Path `$PSScriptRoot 'CheckPerformance.psm1')", "'$module'")
    $bridge = (Join-Path $capture 'TargetedResident.psm1').Replace("'", "''")
    $source = $source.Replace("(Join-Path `$PSScriptRoot 'TargetedResident.psm1')", "'$bridge'")
    $helper = (Join-Path $capture 'CaptureProcesses.cs').Replace("'", "''")
    $source = $source.Replace("(Join-Path `$PSScriptRoot 'CaptureProcesses.cs')", "'$helper'")
    $hostScript = Join-Path $directory 'failing-host.ps1'
    [IO.File]::WriteAllText($hostScript, $source, [Text.UTF8Encoding]::new($false))
}
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = 'regression targeted-compare --original "{0}" --default "{1}" --output "{2}" --observatory-group-dir "{3}" --host-script "{4}" --operation-timeout-seconds 30' -f `
        (Join-Path $directory 'inputs/original.xlsx'), (Join-Path $directory 'inputs/default.xlsx'), (Join-Path $directory 'result'), $group, $hostScript
    if (-not ($CheckPerformance -or $FailOptimization -or $CleanWorkbook)) { $arguments += ' --skip-check-performance' }
    $process = $desktop.Start((Join-Path $repository 'target/debug/excel-shrink-cli.exe'), $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddSeconds(150)
    while (-not $process.WaitForExit(500)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Smoke test exceeded 150 seconds.' }
    }
    $telemetry = Get-Content -LiteralPath (Join-Path $directory 'result/telemetry.json') -Raw | ConvertFrom-Json
    if ($Charts -and $process.ExitCode -eq 0) {
        & python (Join-Path $PSScriptRoot 'verify_charts.py') (Join-Path $directory 'result')
        if ($LASTEXITCODE -ne 0) { throw 'Chart evidence verification failed.' }
    }
    if ($FailCapture -or $FailOptimization -or $FailLateCapture) {
        if ($process.ExitCode -eq 0 -or $telemetry.status -ne 'failed' -or $null -eq $telemetry.finished_at) {
            throw 'The injected failure did not produce a terminal failure snapshot.'
        }
    } elseif ($process.ExitCode -ne 0) { throw "Smoke test failed; inspect $directory/result/failure.json." }
    foreach ($name in @('original', 'default')) {
        if ((Get-FileHash (Join-Path $directory "inputs/$name.xlsx") -Algorithm SHA256).Hash -ne $sourceHashes[$name]) {
            throw "Source input was modified: $name"
        }
    }
    if ($CleanWorkbook -and -not ($FailCapture -or $FailOptimization -or $FailLateCapture)) {
        $report = Get-Content (Join-Path $directory 'result/report.json') -Raw | ConvertFrom-Json
        if ($report.check_performance -ne 'no_optimization_needed') { throw 'Clean workbook was not recognized.' }
        if ($report.check_performance_evidence.optimization_changes.Count -ne 0) { throw 'Unexpected optimization changes.' }
    } elseif ($CheckPerformance -and -not $Charts -and -not $ConditionalFormatting -and -not ($FailCapture -or $FailOptimization -or $FailLateCapture)) {
        & python (Join-Path $PSScriptRoot 'verify_performance_smoke.py') (Join-Path $directory 'result')
        if ($LASTEXITCODE -ne 0) { throw 'Check Performance evidence verification failed.' }
    }
    if ($ConditionalFormatting -and $process.ExitCode -eq 0) {
        & python (Join-Path $PSScriptRoot 'verify_cf.py') (Join-Path $directory 'result') $ConditionalFormatting
        if ($LASTEXITCODE -ne 0) { throw 'Conditional-formatting evidence verification failed.' }
    }
    if ($RecalcCandidate -and -not ($FailCapture -or $FailOptimization -or $FailLateCapture)) {
        & python (Join-Path $PSScriptRoot 'verify_recalc_candidate.py') (Join-Path $directory 'result')
        if ($LASTEXITCODE -ne 0) { throw 'Late recalculation candidate verification failed.' }
    }
    if ($SaveOverlap -and -not ($FailCapture -or $FailOptimization -or $FailLateCapture)) {
        $report = Get-Content (Join-Path $directory 'result/report.json') -Raw | ConvertFrom-Json
        $inputChanges = @($report.transitions | Where-Object label -eq 'default input versus original input')[0].changes
        $overlap = @($inputChanges | Where-Object { $_.location -eq 'cell:A10' -and $_.also_in_original_open_save })
        if ($overlap.Count -ne 1) { throw 'Expected exact opening/saving overlap was not attributed.' }
    }
    if ($FailLateCapture) {
        $failure = Get-Content (Join-Path $directory 'result/failure.json') -Raw | ConvertFrom-Json
        if ($failure.error -notlike '*Injected late capture failure*') { throw 'Unexpected late-failure reason.' }
        $expected = @($telemetry.lanes | Where-Object lifecycle -ne 'unavailable').Count
        if (@($telemetry.lanes | Where-Object pid).Count -ne $expected) { throw 'The late failure did not exercise every enabled Excel state.' }
    }
    if ($FailOptimization) {
        $failure = Get-Content (Join-Path $directory 'result/failure.json') -Raw | ConvertFrom-Json
        if ($failure.error -notlike '*no confirmed outcome*') { throw 'Unexpected failure reason.' }
        if ($telemetry.lanes[2].steps.optimize -ne 'failed') { throw 'Optimization failure was not recorded.' }
    }
    foreach ($lane in $telemetry.lanes) {
        if ($lane.pid -and ($lane.lifecycle -ne 'closed' -or $lane.process_alive -ne $false)) { throw "Owned Excel exit was not confirmed: $($lane.id)" }
    }
    [pscustomobject]@{status=$telemetry.status; candidates=$telemetry.candidate_count; lanes=$telemetry.lanes | Select-Object id,lifecycle,captures_done,captures_total} | ConvertTo-Json -Depth 4
} finally {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previousDesktop
    try { $owned.Dispose() }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        $desktop.Dispose()
    }
}
