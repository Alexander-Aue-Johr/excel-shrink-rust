$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -eq 'Confirm-ResidentClientExit') { Invoke-Expression $function.Extent.Text }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function ExitFixture([int]$After) {
    $process = [pscustomobject]@{ checks=0; after=$After }
    $process | Add-Member ScriptMethod WaitForExit { param($timeout) $this.checks++; return $this.checks -ge $this.after }
    return @{ process=$process }
}

# A job-close request can precede the signal by several checks. The final
# published state must not retain "closing" after the launcher has exited.
$clients = @{ original=(ExitFixture 1); default=(ExitFixture 3); maximum=(ExitFixture 4) }
$metrics = @{}
foreach ($id in $clients.Keys) { $metrics[$id] = @{ stage='capturing'; status='running'; process_alive=$true } }
$pending = @(Confirm-ResidentClientExit $clients $metrics 1000)
Assert ($pending.Count -eq 0) 'Delayed exits were not confirmed.'
foreach ($id in $clients.Keys) {
    Assert ($metrics[$id].stage -eq 'closed' -and $metrics[$id].closed -and -not $metrics[$id].process_alive) 'Confirmed exit retained stale state.'
    Assert ($metrics[$id].exit_confirmed_unix_ms -gt 0) 'Exit confirmation time was not recorded.'
}

# Observed Excel teardown can exceed three seconds after the job is closed.
# The default shared deadline must cover that delay without claiming early exit.
$exitClock = [Diagnostics.Stopwatch]::StartNew()
$delayedProcess = [pscustomobject]@{ clock=$exitClock }
$delayedProcess | Add-Member ScriptMethod WaitForExit {
    param($timeout)
    return $this.clock.ElapsedMilliseconds -ge 3500
}
$clients = @{ one=@{ process=$delayedProcess } }
$metrics = @{ one=@{} }
$pending = @(Confirm-ResidentClientExit $clients $metrics)
Assert ($pending.Count -eq 0) 'The default deadline rejected a delayed process exit.'
Assert ($exitClock.ElapsedMilliseconds -ge 3500) 'Exit was reported before the process signalled.'

$clients = @{}
$metrics = @{}
foreach ($id in 1..8) {
    $name = [string]$id
    $clients[$name] = ExitFixture ([int]::MaxValue)
    $metrics[$name] = @{}
}
$clock = [Diagnostics.Stopwatch]::StartNew()
$pending = @(Confirm-ResidentClientExit $clients $metrics 150)
Assert ($pending.Count -eq 8) 'Unconfirmed exits were lost.'
Assert ($clock.ElapsedMilliseconds -lt 800) 'Exit waits were multiplied by the client count.'
foreach ($id in $clients.Keys) {
    Assert ($metrics[$id].stage -eq 'exit_unconfirmed' -and $null -eq $metrics[$id].process_alive) 'Unknown exit was falsely reported as live or closed.'
}

# Exercise retained process handles, not Excel or a reusable PID lookup.
Add-Type -Path (Join-Path $captureRoot 'CaptureProcesses.cs')
$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 120' -WindowStyle Hidden -PassThru
try {
    [void]$child.Handle
    Assert ([CaptureProcesses]::Stop($child)) 'Owned process did not confirm termination.'
    Assert ($child.WaitForExit(0)) 'Owned process remained alive.'
    Assert ([CaptureProcesses]::Stop($child)) 'Already exited process was not harmless.'
} finally {
    if (-not $child.HasExited) { $child.Kill() }
    $child.Dispose()
}
Write-Host 'PASS: delayed exits, shared cleanup deadline, explicit uncertainty, retained-handle termination.'
