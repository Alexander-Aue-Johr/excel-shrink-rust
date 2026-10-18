# Pure budget evaluation, called by the existing owned-process supervisor.
# Heartbeats do not reset these clocks. No process is created or killed here.
function Get-VVPhaseDeadlineFailure($Parameters,[double]$Elapsed,[string]$Phase,[double]$PhaseElapsed) {
    if($null -eq $Parameters){return $null}
    if($Elapsed -ge [double]$Parameters.workload_timeout_seconds){return [pscustomobject]@{kind='workload';limit=$Parameters.workload_timeout_seconds}}
    $limit=switch($Phase){
        'startup' { [double]$Parameters.startup_timeout_seconds }
        'capture' { [double]$Parameters.capture_timeout_seconds }
        'complete' { 30.0 }
        'result-received' { 30.0 }
        'finished' { 30.0 }
        'failed' { 30.0 }
        'timeout' { 30.0 }
        default { [double]$Parameters.setup_timeout_seconds }
    }
    if($PhaseElapsed -ge $limit){return [pscustomobject]@{kind=$Phase;limit=$limit}}
    return $null
}
