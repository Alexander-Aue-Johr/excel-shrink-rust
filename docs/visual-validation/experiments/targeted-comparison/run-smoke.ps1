param([Parameter(Mandatory=$true)][string]$Output, [switch]$FailCapture)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
$directory = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $directory) { throw 'Use a new smoke-test directory.' }
$null = New-Item -ItemType Directory -Path $directory
& python (Join-Path $PSScriptRoot 'make_smoke_fixtures.py') (Join-Path $directory 'inputs')
if ($LASTEXITCODE -ne 0) { throw 'Fixture creation failed.' }
$group = Join-Path $directory 'control/groups/smoke'
$null = New-Item -ItemType Directory -Path $group -Force
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$previousDesktop = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
$hostScript = Join-Path $capture 'targeted-comparison-host.ps1'
if ($FailCapture) {
    # Fault injection stays in a disposable host copy, after real Excel startup.
    $source = [IO.File]::ReadAllText($hostScript)
    $source = $source.Replace('$session.workbook.Activate()', 'throw "Injected capture failure"')
    $helper = (Join-Path $capture 'CaptureProcesses.cs').Replace("'", "''")
    $source = $source.Replace("(Join-Path `$PSScriptRoot 'CaptureProcesses.cs')", "'$helper'")
    $hostScript = Join-Path $directory 'failing-host.ps1'
    [IO.File]::WriteAllText($hostScript, $source, [Text.UTF8Encoding]::new($false))
}
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $arguments = 'regression targeted-compare --original "{0}" --default "{1}" --output "{2}" --observatory-group-dir "{3}" --host-script "{4}" --operation-timeout-seconds 30' -f `
        (Join-Path $directory 'inputs/original.xlsx'), (Join-Path $directory 'inputs/default.xlsx'), (Join-Path $directory 'result'), $group, $hostScript
    $process = $desktop.Start((Join-Path $repository 'target/debug/excel-shrink-cli.exe'), $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddSeconds(150)
    while (-not $process.WaitForExit(500)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Smoke test exceeded 150 seconds.' }
    }
    $telemetry = Get-Content -LiteralPath (Join-Path $directory 'result/telemetry.json') -Raw | ConvertFrom-Json
    if ($FailCapture) {
        if ($process.ExitCode -eq 0 -or $telemetry.status -ne 'failed' -or $null -eq $telemetry.finished_at) {
            throw 'The injected failure did not produce a terminal failure snapshot.'
        }
    } elseif ($process.ExitCode -ne 0) { throw "Smoke test failed; inspect $directory/result/failure.json." }
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
