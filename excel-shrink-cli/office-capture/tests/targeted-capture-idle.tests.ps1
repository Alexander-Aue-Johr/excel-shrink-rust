$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$root=Join-Path $repo ('.tmp/capture-idle-tests/'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $root|Out-Null
$fake=Join-Path $root 'capture-stub.exe'
Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Diagnostics;
using System.Threading;
public static class CaptureStub {
    public static void Main(string[] args) {
        string marker=Environment.GetEnvironmentVariable("EXCEL_SHRINK_FAKE_PID_FILE");
        if(!String.IsNullOrEmpty(marker))File.WriteAllText(marker,Process.GetCurrentProcess().Id.ToString());
        string mode=Environment.GetEnvironmentVariable("EXCEL_SHRINK_FAKE_CAPTURE_MODE");
        if(String.IsNullOrEmpty(mode)){Thread.Sleep(60000);return;}
        string output=null;
        for(int i=0;i<args.Length-1;i++)if(args[i]=="--output")output=args[i+1];
        Directory.CreateDirectory(output);
        for(int i=1;i<=8;i++){
            string json="{\"status\":\"running\",\"error\":null,\"finished_at\":null,\"lanes\":[],\"updated_at\":"+i+
                ",\"capture_success_sequence\":"+(mode=="capture"?i:0)+"}";
            File.WriteAllText(Path.Combine(output,"telemetry.json"),json);
            Thread.Sleep(350);
        }
    }
}
"@ -OutputAssembly $fake -OutputType ConsoleApplication
$peer=Start-Process $fake -WindowStyle Hidden -PassThru
$results=@()
try {
    foreach($mode in @('heartbeat','capture')){
        $output=Join-Path $root $mode
        $env:EXCEL_SHRINK_FAKE_CAPTURE_MODE=$mode
        $env:EXCEL_SHRINK_FAKE_PID_FILE=Join-Path $root ($mode+'-pid.txt')
        $arguments=@('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'excel-shrink-cli/office-capture/run-targeted-comparison.ps1'),'-Binary',$fake,'-Original','unused.xlsx','-Default','unused.xlsx','-Output',$output,'-ObservatoryGroup',$root,'-CaptureIdleTimeoutSeconds','1')
        $timer=[Diagnostics.Stopwatch]::StartNew()
        $wrapper=Start-Process powershell.exe -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root ($mode+'.stdout')) -RedirectStandardError (Join-Path $root ($mode+'.stderr'))
        $null = $wrapper.Handle
        if(-not $wrapper.WaitForExit(20000)){Stop-Process -Id $wrapper.Id -Force;throw 'Idle watchdog did not return.'}
        $wrapper.Refresh()
        [IO.File]::WriteAllText((Join-Path $root ($mode+'-exit.json')),(@{exit=$wrapper.ExitCode;seconds=$timer.Elapsed.TotalSeconds;has_exited=$wrapper.HasExited}|ConvertTo-Json))
        if($mode -eq 'capture'){
            if($wrapper.ExitCode -ne 0 -or $timer.Elapsed.TotalSeconds -lt 2.5){throw 'Successful captures did not extend the attempt.'}
        } else {
            if($wrapper.ExitCode -eq 0){throw 'Heartbeats kept an idle attempt alive.'}
            $failure=Get-Content (Join-Path $output 'failure.json') -Raw|ConvertFrom-Json
            if($failure.kind -ne 'capture_idle_timeout'){throw 'Idle failure receipt missing.'}
        }
        $ownedId=[int](Get-Content -LiteralPath $env:EXCEL_SHRINK_FAKE_PID_FILE)
        Start-Sleep -Milliseconds 100
        if(Get-Process -Id $ownedId -ErrorAction SilentlyContinue){throw 'Owned child survived.'}
        $peer.Refresh()
        if($peer.HasExited){throw 'Unrelated peer was killed.'}
        $results+=@{mode=$mode;seconds=$timer.Elapsed.TotalSeconds;exit=$wrapper.ExitCode;unrelated_peer_survived=$true}
    }
} finally {
    Remove-Item Env:EXCEL_SHRINK_FAKE_CAPTURE_MODE,Env:EXCEL_SHRINK_FAKE_PID_FILE -ErrorAction SilentlyContinue
    $peer.Refresh()
    if(-not $peer.HasExited){Stop-Process -Id $peer.Id -Force}
}
$results|ConvertTo-Json