$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Get-ResidentControlRoot', 'Get-ResidentOpenTimeoutSeconds', 'Get-ResidentHideExcel', 'Update-ClientActivity', 'Update-ClientStateTimes', 'Update-ClientProcessMetrics', 'Test-ResidentDeadline', 'Update-Resources')) {
        Invoke-Expression $function.Extent.Text
    }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function Update-OtherGroupPause { }
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$root = Join-Path ([IO.Path]::GetTempPath()) ('es-resident-control-' + [Guid]::NewGuid().ToString('N'))
$previousAttempt = $env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID
$previousHide = $env:EXCEL_SHRINK_HIDE_EXCEL
$env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID = $null
$env:EXCEL_SHRINK_HIDE_EXCEL = $null
[void][IO.Directory]::CreateDirectory($root)
try {
    Assert ((Get-ResidentControlRoot 'V:\study\control-room') -eq 'V:\study\control-room') 'Ordinary Windows root changed.'
    Assert ((Get-ResidentControlRoot '\\?\V:\study\control-room') -eq 'V:\study\control-room') 'Extended drive path is not usable by PowerShell providers.'
    Assert ((Get-ResidentControlRoot '\\?\UNC\tower\share\control-room') -eq '\\tower\share\control-room') 'Extended UNC path was not normalized correctly.'
    $relativeRejected = $false
    try { Get-ResidentControlRoot 'relative\study' } catch { $relativeRejected = $true }
    Assert $relativeRejected 'Relative control room paths were accepted.'
    $StartupTimeoutSeconds = 300
    $CaptureTimeoutSeconds = 1800
    $liveControlPath = $null
    $script:liveOpenTimeoutSeconds = $StartupTimeoutSeconds
    $HideExcel = $true
    $script:liveHideExcel = $true
    Assert (Get-ResidentHideExcel) 'Legacy hidden-launch choice changed.'
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 300) 'Legacy startup timeout changed.'
    Assert (Test-ResidentDeadline 'opening' 301) 'Legacy opening deadline stopped expiring.'
    Assert (-not (Test-ResidentDeadline 'opening' 299)) 'Legacy opening expired early.'
    Assert (Test-ResidentDeadline 'session' 1801) 'Legacy session deadline stopped expiring.'

    $liveControlPath = Join-Path $root 'control.json'
    [IO.File]::WriteAllText($liveControlPath, '{"hide_excel":false}')
    Assert (-not (Get-ResidentHideExcel)) 'The next launch ignored the live visibility choice.'
    foreach ($invalid in @('{', '{}', '{"hide_excel":"true"}', '{"hide_excel":1}')) {
        [IO.File]::WriteAllText($liveControlPath, $invalid)
        Assert (-not (Get-ResidentHideExcel)) 'Invalid controls changed the last valid visibility choice.'
    }
    [IO.File]::WriteAllText($liveControlPath, '{"hide_excel":true}')
    Assert (Get-ResidentHideExcel) 'Hidden mode cannot be enabled for the next launch.'
    $env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID = 'frozen-attempt'
    $env:EXCEL_SHRINK_HIDE_EXCEL = '0'
    Assert (-not (Get-ResidentHideExcel)) 'A dashboard change altered the recorded group visibility setting.'
    $env:EXCEL_SHRINK_HIDE_EXCEL = '1'
    [IO.File]::WriteAllText($liveControlPath, '{"hide_excel":false}')
    Assert (Get-ResidentHideExcel) 'A frozen hidden group was switched to normal for its next client.'
    $env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID = $null
    $env:EXCEL_SHRINK_HIDE_EXCEL = $null
    Assert (-not (Get-ResidentHideExcel)) 'Standalone launches stopped following live visibility controls.'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":900,"paused":true}')
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 900) 'Live startup timeout did not replace the CLI default.'
    Assert (-not (Test-ResidentDeadline 'opening' 850)) 'Live opening expired against the obsolete deadline.'
    Assert (Test-ResidentDeadline 'opening' 901) 'Pausing captures also disabled the active opening deadline.'
    Assert (-not (Test-ResidentDeadline 'session' 1000000)) 'A fixed session deadline would kill a user-paused live capture.'

    # Changing the dashboard timeout affects the already-running opening clock.
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":3600}')
    Assert (-not (Test-ResidentDeadline 'opening' 1100)) 'An in-flight opening ignored an extended timeout.'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":100}')
    Assert (Test-ResidentDeadline 'opening' 1100) 'An in-flight opening ignored a shortened timeout.'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":9000}')
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'The live timeout retained an old CLI upper limit.'
    foreach ($invalid in @('{', '{}', '{"open_timeout_seconds":-1}', '{"open_timeout_seconds":1.5}', '{"open_timeout_seconds":"10"}', '{"open_timeout_seconds":true}')) {
        [IO.File]::WriteAllText($liveControlPath, $invalid)
        Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'Invalid control data shortened the last valid opening deadline.'
    }
    Remove-Item -LiteralPath $liveControlPath
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'A transiently missing snapshot shortened the opening deadline.'

    $metrics = @{ clients=@{ original=[ordered]@{status='ready'; heartbeat_count=0; last_heartbeat_at=$null; peak_private_bytes=0} } }
    $health = '{"clients":{"original":{"heartbeat_count":7,"last_heartbeat_at":"2026-10-24T10:00:00Z","last_activity_at":"2026-10-24T10:00:01Z","active_command":"capture-1","error":null}}}' | ConvertFrom-Json
    Update-ClientActivity $health
    Assert ($metrics.clients.original.heartbeat_count -eq 7) 'Real heartbeat count was not published.'
    Assert ($metrics.clients.original.stage -eq 'executing_job') 'The current capture job was not published.'
    $health.clients.original.last_activity_at = '2026-10-24T10:00:10Z'
    Update-ClientActivity $health
    Assert ($metrics.clients.original.last_heartbeat_at -eq '2026-10-24T10:00:00Z') 'Ordinary activity invented a heartbeat.'
    Update-ClientStateTimes
    $stateSince = $metrics.clients.original.stage_since
    $lastEvent = $metrics.clients.original.last_launcher_event.at
    Update-ClientStateTimes
    Assert ($metrics.clients.original.stage_since -eq $stateSince) 'Sampling reset the current state duration.'
    Assert ($metrics.clients.original.last_launcher_event.at -eq $lastEvent) 'Sampling invented a state change.'
    $health.clients.original | Add-Member -NotePropertyName monitor -NotePropertyValue ([pscustomobject]@{state='capturing';state_since='2026-10-24T09:00:00Z';last_progress_at='2026-10-24T09:30:00Z'})
    Update-ClientActivity $health
    Update-ClientStateTimes
    Assert ($metrics.clients.original.stage_since -eq '2026-10-24T09:00:00Z') 'The original protocol state timestamp was lost.'
    Assert ($metrics.clients.original.monitor.last_progress_at -eq '2026-10-24T09:30:00Z') 'Activity was substituted for actual progress.'
    $health.clients.original.error = @{ code='InvalidOperationInCellEditMode'; message='Exit edit mode' }
    Update-ClientActivity $health
    Assert ($metrics.clients.original.stage -eq 'blocked_edit_mode') 'Excel edit mode is not visible as a distinct blockage.'
    $process = [Diagnostics.Process]::GetCurrentProcess()
    $clients = @{ original=@{ process=$process; closed=$false } }
    Update-ClientProcessMetrics
    Assert ($metrics.clients.original.process_alive -eq $true) 'A quiet heartbeat made the native process look dead.'
    $process.Dispose()

    $liveOwner = @{HasExited=$true}
    $caught = $null
    try { Update-Resources } catch { $caught = $_.Exception.Message }
    Assert ($caught -eq 'Control room coordinator exited.') 'Live waiting masked coordinator death.'
    Write-Host 'PASS: deadlines, queued opens, retained controls, hidden launches, actual heartbeats, process liveness, edit-mode diagnosis, coordinator death.'
} finally {
    $env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID = $previousAttempt
    $env:EXCEL_SHRINK_HIDE_EXCEL = $previousHide
    $resolved = [IO.Path]::GetFullPath($root)
    $temporary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temporary, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'es-resident-control-*') {
        throw 'Unexpected control-test directory.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
