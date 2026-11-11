$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedResident.psm1') -Force -DisableNameChecking
$module = Get-Module TargetedResident
$root = Join-Path ([IO.Path]::GetTempPath()) ('resident-exit-' + [Guid]::NewGuid())
$null = [IO.Directory]::CreateDirectory($root)
try {
    & $module {
        param($Root)
        function Start-TargetedJob {
            param($Command, $Index)
            return @{entry=$sessions[$Command.lane]; index=$Index; command=$Command}
        }
        $sessions.Clear()
        $sessions['O'] = @{
            path=(Join-Path $Root 'original.xlsx')
            session=$Root
            launcher=[pscustomobject]@{HasExited=$true}
        }
        $metricsPath = Join-Path $Root 'metrics.json'
        $guardMessage = 'Resident RAM guard: system commit headroom 740 MiB; required reserve 8192 MiB.'
        foreach ($fixture in @(
            @{json=(@{error=$guardMessage} | ConvertTo-Json); expected=$guardMessage},
            @{json='{}'; expected='Targeted resident exited without a response.'},
            @{json='{invalid'; expected='Resident diagnostic could not be read.'},
            @{json=$null; expected='Targeted resident exited without a response.'}
        )) {
            if ($null -eq $fixture.json) {
                [IO.File]::Delete($metricsPath)
            } else {
                [IO.File]::WriteAllText($metricsPath, $fixture.json)
            }
            $failureMessage = $null
            try { Invoke-TargetedJobs @(@{lane='O';kind='capture'}) }
            catch { $failureMessage = $_.Exception.Message }
            if (-not $failureMessage -or -not $failureMessage.Contains($fixture.expected)) {
                throw "Resident exit diagnostic lost expected text: $($fixture.expected)"
            }
        }
        $script:responseWatcher.Dispose()
        $script:responseWatcher = $null
        $sessions.Clear()
    } $root
} finally {
    # The fixture owns only this generated file and empty directory.
    [IO.File]::Delete((Join-Path $root 'metrics.json'))
    [IO.Directory]::Delete($root)
}
'Resident exit diagnostic checks passed.'
