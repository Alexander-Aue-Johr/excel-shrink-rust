function Get-TargetedProcessAlive {
    <#
    .SYNOPSIS
    Observe a recorded process identity without controlling that process.
    .DESCRIPTION
    Returns null when identity or access is insufficient. A reused PID is not the
    recorded process; neither a missing birth time nor access failure proves exit.
    #>
    param(
        [Parameter(Mandatory=$true)][int]$ProcessId,
        [Nullable[long]]$StartedAt
    )
    if ($null -eq $StartedAt) { return $null }
    $observed = $null
    try {
        $observed = [Diagnostics.Process]::GetProcessById($ProcessId)
        $birth = ([DateTimeOffset]$observed.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
        return $birth -eq $StartedAt
    } catch [ArgumentException] {
        return $false
    } catch {
        return $null
    } finally {
        if ($null -ne $observed) { $observed.Dispose() }
    }
}

Export-ModuleMember -Function Get-TargetedProcessAlive
