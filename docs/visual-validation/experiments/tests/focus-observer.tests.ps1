# No Excel, COM, clipboard or real process. Runs before a focused capture on Windows.
param()
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\_shared\office-addin\powershell\focus-observer.ps1')
$dir=Join-Path $env:TEMP ('vv-observer-test-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($dir)
try {
    function Write-VVJson([string]$Path,$Value) {
        [IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 15),[Text.UTF8Encoding]::new($false))
    }
    $o=New-VVFocusObserver $dir 'test.xlsx' @{}
    $fake=[pscustomobject]@{Id=7;HasExited=$true}
    $fake | Add-Member ScriptMethod Refresh { }
    $o.Process=$fake
    Detach-VVFocusProcess $o
    if($null -ne $o.Process -or $o.State.process.status -cne 'process-exited'){throw 'Observer did not detach an exited process'}
    $fake | Add-Member ScriptMethod Refresh { throw 'Disposed object accessed' } -Force
    Save-VVFocusObserver $o -Force
    if($o.State.process.status -cne 'process-exited' -or $o.State.process.last_successful_sample.pid -ne 7){throw 'Observer lost its final identity'}
    Complete-VVFocusResult $o 'partial'
    if($o.State.capture_status -cne 'partial' -or $o.State.phase -cne 'result-received'){throw 'Result transport was confused with capture success'}
    Set-VVFocusPhase $o 'finished'
    if($o.State.capture_status -cne 'partial'){throw 'Final lifecycle state changed the capture outcome'}
    $missing=Get-VVFocusProcessSnapshot $null
    if($missing.available){throw 'An absent process was fabricated'}
    Write-Host 'OBSERVER PREFLIGHT: detach, retained identity and outcome separation passed (no Excel).'
} finally {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
