$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'Launch.psm1') -Force
$root=Join-Path ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))) ('.tmp/stop-tests/'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory((Join-Path $root 'control-room'))
$helper=Join-Path $root 'excel-shrink-cli.exe'
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
public class StopFixture {
 public static void Main(string[] args) {
  using(var file=new FileStream(Path.Combine(args[0], ".run.lock"),FileMode.OpenOrCreate,FileAccess.ReadWrite,FileShare.ReadWrite)) {
   file.Lock(0,1); Thread.Sleep(60000);
  }
 }
}
'@ -OutputAssembly $helper -OutputType ConsoleApplication
function Assert($Value,$Message){if(-not $Value){throw $Message}}
$child=Start-Process -FilePath $helper -ArgumentList ('"'+$root+'"') -WindowStyle Hidden -PassThru
try {
    $until=[DateTime]::UtcNow.AddSeconds(5)
    while(-not (Test-StudyOutputBusy $root)){
        if([DateTime]::UtcNow -gt $until){throw 'Fixture did not acquire the output lock.'}
        Start-Sleep -Milliseconds 50
    }
    $started=[DateTimeOffset]::new($child.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    $queue=[pscustomobject]@{runner_pid=$child.Id;runner_started_unix_ms=$started;phase='comparing';groups=@([pscustomobject]@{active=$true})}
    $path=Join-Path $root 'control-room/queue.json'
    Save-StudyJson $path $queue
    $requested=Get-StudyRunStatus $root
    Assert ($requested.Process.Id -eq $child.Id) 'Owned runner not recognized.'
    Complete-StudyLegacyStop $root $requested
    Assert (-not $child.HasExited) 'Active workbook was killed.'
    $queue.runner_started_unix_ms=$started-1000
    Save-StudyJson $path $queue
    Assert ($null -eq (Get-StudyRunStatus $root).Process) 'Reused PID was accepted.'
    $queue.runner_started_unix_ms=$started;$queue.phase='paused'
    Save-StudyJson $path $queue
    Complete-StudyLegacyStop $root $requested
    Assert (-not $child.HasExited) 'Active group was killed during a pause.'
    $queue.groups=@();Save-StudyJson $path $queue
    Complete-StudyLegacyStop $root $requested
    Assert ($child.WaitForExit(5000)) 'Drained legacy runner did not exit.'
    Assert ((Get-StudyRunStatus $root).Ended) 'Stopped runner still blocks updates.'
    'Stop lifecycle: active work retained, PID reuse rejected, drained runner stopped, lock released.'
} finally {
    $child.Refresh()
    if(-not $child.HasExited){$child | Stop-Process -Force}
}
