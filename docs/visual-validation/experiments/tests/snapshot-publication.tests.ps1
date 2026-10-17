# Real Windows filesystem/concurrency checks. No Office, COM, WPF or clipboard.
param([string]$ProbeRoot=$env:TEMP,[string]$ResultPath)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\_shared\reporting.ps1')
Initialize-VVReportIO
$probe=Join-Path ([IO.Path]::GetFullPath($ProbeRoot)) ('.snapshot-probe-'+[Guid]::NewGuid().ToString('N').Substring(0,8))
[void][IO.Directory]::CreateDirectory($probe)
$checks=[Collections.Generic.List[string]]::new();$child=$null;$held=$null;$release=$null
$receipt=[ordered]@{schema='vv-snapshot-preflight-v1';status='running';checks=@();probe_root=$ProbeRoot;error=$null;policy=[VVSnapshotFile]::Policy}
function Assert-Snapshot([bool]$Condition,[string]$Message){if(-not $Condition){throw $Message}}
function Snapshot-Cause($ErrorRecord){$e=$ErrorRecord.Exception;while($null -ne $e -and $e.GetType().FullName -ne 'VVSnapshotIOException'){$e=$e.InnerException};return $e}
try {
    if($null -eq ('VVSnapshotTestRelease' -as [type])) {
        Add-Type -ReferencedAssemblies @('System.dll') -TypeDefinition @'
using System;
using System.Threading;
public static class VVSnapshotTestRelease {
    public static Thread Later(IDisposable handle, int milliseconds) {
        var thread = new Thread(delegate() { Thread.Sleep(milliseconds); handle.Dispose(); });
        thread.IsBackground = true; thread.Start(); return thread;
    }
}
'@
    }
    $path=Join-Path $probe 'state.json'
    $isWslPath=$path -match '^\\\\(?:wsl\.localhost|wsl\$)\\'
    Write-VVJson $path @{sequence=0;payload=('x'*512);text='Muenchen / \u03b1'}
    Assert-Snapshot ((Read-VVJson $path).sequence -eq 0) 'First snapshot was not readable'
    Assert-Snapshot ($null -eq (Read-VVJson (Join-Path $probe 'absent.json'))) 'Absent snapshot was not distinguished'
    $checks.Add('create and genuinely absent snapshot')

    # A reader holds the old file open. Replacement must not change its bytes or fail solely due to our reader.
    $old=[VVSnapshotFile]::ReadTextOrNull($path)
    $held=[VVReportIO]::OpenRead($path)
    Write-VVJson $path @{sequence=1;payload=('x'*512)}
    $reader=[IO.StreamReader]::new($held,[Text.Encoding]::UTF8,$true)
    try{$oldThroughHandle=$reader.ReadToEnd()}finally{$reader.Dispose();$held=$null}
    Assert-Snapshot ($oldThroughHandle -ceq $old) 'A published snapshot was overwritten in place'
    Assert-Snapshot ((Read-VVJson $path).sequence -eq 1) 'Replacement was not published'
    $checks.Add('open reader retains old bytes while new snapshot is published')

    $held=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
    $before=[VVSnapshotFile]::ReadRetries;$release=[VVSnapshotTestRelease]::Later($held,250)
    $r=Read-VVJson $path
    $release.Join();$release=$null;$held=$null
    Assert-Snapshot ($r.sequence -eq 1 -and [VVSnapshotFile]::ReadRetries -gt $before) 'Transient read sharing conflict was not retried'
    $checks.Add('short exclusive read conflict recovers')

    $held=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $before=[VVSnapshotFile]::PublishRetries;$release=[VVSnapshotTestRelease]::Later($held,250)
    Write-VVJson $path @{sequence=2;payload=('x'*512)}
    $release.Join();$release=$null;$held=$null
    Assert-Snapshot ((Read-VVJson $path).sequence -eq 2 -and ($isWslPath -or [VVSnapshotFile]::PublishRetries -gt $before)) 'Short replacement conflict did not recover'
    $checks.Add('short rename sharing conflict recovers')

    $held=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $failure=$null;$old=[VVSnapshotFile]::ReadTextOrNull($path)
    try{Write-VVJson $path @{sequence=3;payload='pending'}}catch{$failure=Snapshot-Cause $_}
    if($isWslPath){
        $reader=[IO.StreamReader]::new($held,[Text.Encoding]::UTF8,$true)
        try{$oldThroughHandle=$reader.ReadToEnd()}finally{$reader.Dispose();$held=$null}
        Assert-Snapshot ($null -eq $failure -and $oldThroughHandle -ceq $old -and (Read-VVJson $path).sequence -eq 3) 'WSL rename did not preserve the held old snapshot'
        $checks.Add('WSL rename preserves the held old snapshot')
    }else{
    $held.Dispose();$held=$null
    Assert-Snapshot ($null -ne $failure -and $failure.Operation -eq 'publish' -and $failure.Attempts -eq 7) 'Permanent conflict did not fail within the bounded retry count'
    Assert-Snapshot ((Read-VVJson $path).sequence -eq 2) 'Failed publication changed the previous snapshot'
    Assert-Snapshot ($failure.PendingComplete -and (Read-VVJson $failure.PendingPath).sequence -eq 3) 'Failed publication did not retain its complete staged data'
    $checks.Add('permanent conflict preserves old snapshot and complete staged evidence')
    }

    [void][VVSnapshotFile]::WriteText($path,'{not valid json')
    $rejected=$false;try{$null=Read-VVJson $path}catch{$rejected=$true}
    Assert-Snapshot $rejected 'Malformed JSON was treated as missing or successful'
    $checks.Add('invalid JSON remains an error')
    Write-VVJson $path @{sequence=0;payload=('x'*512)}

    # A separate PowerShell process writes while this process polls the exact same destination.
    $reporting=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\_shared\reporting.ps1'))
    $body="`$ErrorActionPreference='Stop'; . '"+$reporting.Replace("'","''")+"'; for(`$i=1;`$i -le 18;`$i++){Write-VVJson '"+$path.Replace("'","''")+"' @{sequence=`$i;payload=('x'*512)};Start-Sleep -Milliseconds 10}"
    $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($body))
    $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $child=Start-Process -FilePath $exe -ArgumentList @('-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand',$encoded) -NoNewWindow -PassThru `
        -RedirectStandardOutput (Join-Path $probe 'writer.out') -RedirectStandardError (Join-Path $probe 'writer.err')
    $clock=[Diagnostics.Stopwatch]::StartNew();$reads=0
    while(-not $child.WaitForExit(10)){
        if($clock.Elapsed.TotalSeconds -gt 90){throw 'Snapshot concurrency probe exceeded 90 seconds'}
        $v=Read-VVJson $path
        Assert-Snapshot ($null -ne $v -and $v.sequence -ge 0 -and $v.sequence -le 18 -and $v.payload -ceq ('x'*512)) 'Concurrent reader saw incomplete JSON'
        $reads++
    }
    $child.Refresh()
    # Start-Process can return a process wrapper without an ExitCode over WSL
    # interop. The final sequence proves the child completed all writes.
    if($null -ne $child.ExitCode){
        Assert-Snapshot ($child.ExitCode -eq 0) ('Snapshot child failed (exit '+$child.ExitCode+'): '+[IO.File]::ReadAllText((Join-Path $probe 'writer.err')))
    }
    Assert-Snapshot ((Read-VVJson $path).sequence -eq 18 -and $reads -gt 0) 'Concurrent final snapshot was not read'
    $checks.Add('separate Windows PowerShell writer and polling reader')

    . (Join-Path $PSScriptRoot '..\_shared\office-addin\powershell\asset-preflight.ps1')
    $id='1234567890abcdef1234567890abcdef'
    foreach($family in @('range-image','shape-bounds','range-image-height-limit','range-image-calibration')){
        $cfg=[pscustomobject]@{host_run_id=$id;experiment=$family;host_kind='dotnet';capture_run_id=$null;diagnostic_run_id=$null}
        if($family -eq 'range-image'){$cfg.capture_run_id=$id}
        if($family -like 'range-image-*'){$cfg.diagnostic_run_id='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'}
        Assert-VVHostIdentity $cfg $id $family 'dotnet'
        $cfg.host_run_id='different';$rejected=$false;try{Assert-VVHostIdentity $cfg $id $family 'dotnet'}catch{$rejected=$true}
        Assert-Snapshot $rejected ('Wrong launch identity was accepted for '+$family)
    }
    $cfg=[pscustomobject]@{host_run_id=$id;experiment='range-image';host_kind='rust';capture_run_id=$id}
    Assert-VVHostIdentity $cfg $id 'range-image' 'rust'
    $checks.Add('per-family identities accepted and stale identities rejected')
    $receipt.status='complete'
    Write-Host ('SNAPSHOT PREFLIGHT: '+$checks.Count+' filesystem/identity checks passed; no Excel instance was opened.')
} catch {
    $receipt.status='failed';$receipt.error=$_.Exception.ToString();throw
} finally {
    if($null -ne $release){$release.Join()}
    if($null -ne $held){$held.Dispose()}
    if($null -ne $child){if(-not $child.HasExited){try{$child.Kill();[void]$child.WaitForExit(5000)}catch{}};$child.Dispose()}
    $receipt.checks=@($checks);$receipt.finished_utc=[DateTime]::UtcNow.ToString('o')
    if($ResultPath){try{Write-VVJson $ResultPath $receipt}catch{
        if($receipt.status -eq 'complete'){throw}
        Write-Warning ('Snapshot preflight receipt could not be published: '+$_.Exception.Message)
    }}
    # Only this test's private scratch directory; never a report or input directory.
    if($receipt.status -eq 'complete'){Remove-Item -LiteralPath $probe -Recurse -Force -ErrorAction SilentlyContinue}
    else{Write-Host ('SNAPSHOT TEST EVIDENCE: '+$probe)}
}
