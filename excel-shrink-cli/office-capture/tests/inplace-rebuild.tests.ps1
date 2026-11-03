$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedResident.psm1') -Force -DisableNameChecking
$module = Get-Module TargetedResident
& $module {
    $sessions.Clear()
    $fake = [pscustomobject]@{Id=1234;HasExited=$false}
    $fake | Add-Member ScriptMethod Refresh {}
    $entry = @{process=$fake;path='before.xlsx';staged='owned-stage.xlsx';sequence=7}
    $sessions['SD'] = $entry
    $result = Move-TargetedWorkbookLane @{lane='SD';target_lane='SDR';output='after.xlsx'}
    if ($result.pid -ne 1234 -or $result.reopened -ne $false -or $sessions.ContainsKey('SD')) {
        throw 'Transition did not transfer the existing process.'
    }
    if (-not [object]::ReferenceEquals($entry,$sessions['SDR']) -or $entry.staged -ne 'owned-stage.xlsx' -or $entry.sequence -ne 7 -or $entry.path -ne 'after.xlsx') {
        throw 'Transition changed resident identity or failed to retain the old output.'
    }
    $failed=$false
    try { Move-TargetedWorkbookLane @{lane='SD';target_lane='SDR';output='again.xlsx'} } catch { $failed=$true }
    if (-not $failed) { throw 'A duplicate transition was accepted.' }
    $sessions.Clear()
    $fake.HasExited=$true
    $sessions['SD']=$entry
    $failed=$false
    try { Move-TargetedWorkbookLane @{lane='SD';target_lane='SDR';output='dead.xlsx'} } catch { $failed=$true }
    if (-not $failed -or -not $sessions.ContainsKey('SD')) { throw 'An exited process was adopted.' }
    $sessions.Clear()
}
'In-place lane ownership tests passed.'