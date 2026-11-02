$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedResident.psm1') -Force -DisableNameChecking
$module = Get-Module TargetedResident
& $module {
    $script:openingEvents = New-Object 'Collections.Generic.List[string]'
    function Start-TargetedWorkbook {
        param($Command, $Owned)
        $script:openingEvents.Add('start-' + $Command.lane)
        return @{lane=$Command.lane}
    }
    function Wait-TargetedWorkbook {
        param($Command, $entry)
        if ($script:openingEvents.Count -lt 2) { throw 'Readiness waited before both launches.' }
        $script:openingEvents.Add('ready-' + $Command.lane)
        return @{pid=if ($entry.lane -eq 'O') { 101 } else { 102 }}
    }
    $result = Open-TargetedWorkbooks @{jobs=@(@{lane='O'}, @{lane='SD'})} $null
    if (($script:openingEvents -join ',') -ne 'start-O,start-SD,ready-O,ready-SD') {
        throw 'Opening order did not overlap launches.'
    }
    if ($result.results[0].pid -ne 101 -or $result.results[1].pid -ne 102) {
        throw 'Opening results lost their lane order.'
    }
    $rejected = $false
    try { Open-TargetedWorkbooks @{jobs=@(@{lane='O'}, @{lane='O'})} $null }
    catch { $rejected = $true }
    if (-not $rejected -or $script:openingEvents.Count -ne 4) {
        throw 'Duplicate lanes must be rejected before any launch.'
    }
}
Write-Output 'Parallel opening dispatch and duplicate rejection passed.'
