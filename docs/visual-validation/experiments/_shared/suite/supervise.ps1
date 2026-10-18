param([Parameter(Mandatory=$true)][string]$SpecPath)
$ErrorActionPreference='Stop'
$utf8=[Text.UTF8Encoding]::new($false)
$spec=[IO.File]::ReadAllText($SpecPath,$utf8)|ConvertFrom-Json
$root=[IO.Path]::GetFullPath($spec.case_root)
$expected=[IO.Path]::GetFullPath((Join-Path $spec.suite_root ('cases\'+$spec.job.id)))
if($root -ne $expected -or $spec.job.id -notmatch '^\d{6,8}-[a-z0-9-]+$' -or $spec.release_token -notmatch '^[a-f0-9]{32}$'){throw 'Invalid decision worker identity'}
. (Join-Path $spec.repo_root 'docs\visual-validation\experiments\_shared\reporting.ps1')
$process=$null;$owner=$null;$clock=[Diagnostics.Stopwatch]::StartNew();$postStarted=$null
$receipt=[ordered]@{schema='vv-decision-supervisor-v1';status='failed';error=$null;safe_to_continue=$false;elapsed_seconds=0;workload_timeout_seconds=$spec.workload_timeout_seconds;phase='startup';owned_excel=@();native_exit=$null}
. (Join-Path $PSScriptRoot 'deadlines.ps1')
$focusPhase='setup';$focusPhaseStarted=0.0
$owned=@{};$preexisting=@(Get-Process -Name EXCEL -ErrorAction SilentlyContinue|ForEach-Object{$_.Id})
$priorToken=$env:VV_DECISION_OWNER_TOKEN;$priorCase=$env:VV_DECISION_CASE_ROOT;$priorExisting=$env:VV_DECISION_PREEXISTING_EXCEL
function Assert-DecisionIdle {
    if(@(Get-Process -Name EXCEL -ErrorAction SilentlyContinue).Count){throw 'Excel is still running. No unrelated instance will be terminated.'}
    $ports=@([Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners()|Where-Object{$_.Port -in @(3000,3001)})
    if($ports.Count){throw 'A capture port is occupied. No unrelated listener will be terminated.'}
}
function Attach-DecisionExcelOwners {
    foreach($f in @(Get-ChildItem -LiteralPath $root -File | Where-Object {$_.Name -like 'owned-excel-*.json' -or $_.Name -eq 'lab-owner.json'})) {
        $e=Read-VVJson $f.FullName
        if($null -eq $e -or $e.owner_token -cne $spec.release_token){continue}
        $identity=([string]$e.pid)+':'+([string]$e.start_ticks)
        if($owned.ContainsKey($identity)){continue}
        if($preexisting -contains [int]$e.pid){throw 'COM owner record refers to a pre-existing Excel instance'}
        $p=$null
        try {
            $p=[Diagnostics.Process]::GetProcessById([int]$e.pid)
            if($p.HasExited){continue}
            if($p.ProcessName -ine 'EXCEL' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -cne [string]$e.start_ticks){throw 'COM process identity changed'}
            $owner.Assign($p);$owned[$identity]=$e
        } catch [ArgumentException] { } finally {if($null -ne $p){$p.Dispose()}}
    }
}
try {
    Assert-DecisionIdle
    if([int]$spec.controller_parent_pid -le 0 -or $null -eq (Get-Process -Id ([int]$spec.controller_parent_pid) -ErrorAction SilentlyContinue)){throw 'Decision session controller is not alive'}
    Add-Type -Path (Join-Path $spec.repo_root 'docs\visual-validation\experiments\_shared\suite\SuiteRuntime.cs') -ReferencedAssemblies @('System.dll') -ErrorAction Stop
    $spec.parent_pid=$PID
    Write-VVJson $SpecPath $spec
    Write-VVJson (Join-Path $root 'run.json') @{schema='vv-report-v1';id=$spec.job.id;experiment=$spec.job.candidate.label;status='queued';owner_pid=0;started_utc=[DateTime]::UtcNow.ToString('o');report_directory=$root;suite_parent=(Split-Path $spec.suite_root -Leaf);arguments=@{Candidate=$spec.job.candidate_id;Stage=$spec.job.stage;Input=$spec.job.workbook}}
    $env:VV_DECISION_OWNER_TOKEN=$spec.release_token;$env:VV_DECISION_CASE_ROOT=$root;$env:VV_DECISION_PREEXISTING_EXCEL=($preexisting -join ',')
    $worker=Join-Path $spec.repo_root 'docs\visual-validation\experiments\_shared\suite\worker.ps1'
    $command="& '"+$worker.Replace("'","''")+"' -SpecPath '"+$SpecPath.Replace("'","''")+"'"
    $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $powershell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $owner=New-Object VVSuiteJob
    $process=Start-Process -FilePath $powershell -NoNewWindow -ArgumentList @('-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-OutputFormat','Text','-EncodedCommand',$encoded) `
        -WorkingDirectory $env:TEMP -RedirectStandardOutput (Join-Path $root 'worker.stdout.log') -RedirectStandardError (Join-Path $root 'worker.stderr.log') -PassThru
    try{$owner.Assign($process)}catch{try{$process.Kill()}catch{};throw}
    [IO.File]::WriteAllText((Join-Path $root '.go'),$spec.release_token,$utf8)
    $receipt.phase='workload'
    while(-not $process.WaitForExit(500)) {
        Attach-DecisionExcelOwners
        if(Test-Path -LiteralPath (Join-Path $root '.cancel')){$receipt.status='cancelled';$receipt.error='Controller cancelled or exhausted session budget';break}
        if($null -eq (Get-Process -Id ([int]$spec.controller_parent_pid) -ErrorAction SilentlyContinue)){$receipt.status='cancelled';$receipt.error='Session controller exited';break}
        $lease=Get-Item -LiteralPath (Join-Path $root '.lease') -ErrorAction SilentlyContinue
        if($clock.Elapsed.TotalSeconds -gt 60 -and ($null -eq $lease -or ([DateTime]::UtcNow-$lease.LastWriteTimeUtc).TotalSeconds -gt 60)){$receipt.status='cancelled';$receipt.error='Campaign heartbeat lease expired';break}
        $phase=Read-VVJson (Join-Path $root 'case-phase.json')
        $life=if($null -ne $spec.focus_parameters){Read-VVJson (Join-Path $root 'capture-lifecycle.json')}else{$null}
        $focusFinished=($null -ne $life -and $life.schema -eq 'vv-focus-lifecycle-v1' -and $life.phase -in @('complete','failed','timeout'))
        if(($null -ne $phase -and $phase.phase -in @('postprocess','report')) -or $focusFinished) {
            if($null -eq $postStarted){$postStarted=$clock.Elapsed.TotalSeconds}
            $receipt.phase=if($null -ne $phase -and $phase.phase -in @('postprocess','report')){$phase.phase}else{'postprocess'}
            if($clock.Elapsed.TotalSeconds-$postStarted -ge [int]$spec.postprocess_timeout_seconds){$receipt.status='timeout';$receipt.error='Post-processing timeout; captured data retained';break}
        } elseif($null -ne $spec.focus_parameters) {
            $observed=if($null -ne $life -and $life.schema -eq 'vv-focus-lifecycle-v1'){[string]$life.phase}else{'setup'}
            if($observed -cne $focusPhase){$focusPhase=$observed;$focusPhaseStarted=$clock.Elapsed.TotalSeconds}
            $receipt.phase=$focusPhase
            $receipt.focus_lifecycle=$life
            $failure=Get-VVPhaseDeadlineFailure $spec.focus_parameters $clock.Elapsed.TotalSeconds $focusPhase ($clock.Elapsed.TotalSeconds-$focusPhaseStarted)
            if($null -ne $failure){$receipt.status='timeout';$receipt.timeout_kind=$failure.kind;$receipt.error=('Focused '+$failure.kind+' deadline exceeded ('+$failure.limit+' s); existing evidence retained.');break}
        } elseif($clock.Elapsed.TotalSeconds -ge [int]$spec.workload_timeout_seconds) {
            $receipt.status='timeout';$receipt.error='Baseline-relative workload deadline exceeded; censored, not successful runtime';break
        }
        if($clock.Elapsed.TotalSeconds -ge [int]$spec.total_remaining_seconds){$receipt.status='timeout';$receipt.error='Overall campaign session budget reached';break}
    }
    if(-not $process.HasExited){
        Attach-DecisionExcelOwners
        $owner.Terminate();[void]$process.WaitForExit(10000)
    } else {
        $process.Refresh();$receipt.native_exit=$process.ExitCode
        $metrics=Read-VVJson (Join-Path $root 'case-metrics.json')
        if($null -eq $metrics){$receipt.status='failed';$receipt.error='Worker exited without case metrics'}
        else{$receipt.status=$metrics.status;$receipt.error=$metrics.invocation_error;if($process.ExitCode -ne 0 -and $receipt.status -eq 'complete'){$receipt.status='failed';$receipt.error='Worker returned a nonzero exit code'}}
    }
    if($null -ne $spec.focus_parameters){
        $life=Read-VVJson (Join-Path $root 'capture-lifecycle.json');$receipt.focus_lifecycle=$life
        if($null -ne $life -and $life.timeout_kind){$receipt.status='timeout';$receipt.timeout_kind=$life.timeout_kind;$receipt.error=$life.error}
    }
    Attach-DecisionExcelOwners
} catch {
    $receipt.status='failed';$receipt.error=$_.Exception.Message
    $cause=$_.Exception
    while($null -ne $cause -and $cause.GetType().FullName -ne 'VVSnapshotIOException'){$cause=$cause.InnerException}
    if($null -ne $cause){
        $receipt.failure_category='report-io'
        $receipt.report_io_failure=@{operation=$cause.Operation;native_code=$cause.NativeCode;attempts=$cause.Attempts;
            target=$cause.TargetPath;pending=$cause.PendingPath;pending_complete=$cause.PendingComplete}
    }
}
finally {
    if($null -ne $owner){try{$owner.Dispose()}catch{}}
    if($null -ne $process){$process.Dispose()}
    $receipt.elapsed_seconds=$clock.Elapsed.TotalSeconds;$receipt.owned_excel=@($owned.Values)
    $cleanup=[Diagnostics.Stopwatch]::StartNew()
    do {try{Assert-DecisionIdle;$receipt.safe_to_continue=$true}catch{$receipt.safe_to_continue=$false};if(-not $receipt.safe_to_continue){Start-Sleep -Milliseconds 500}}while(-not $receipt.safe_to_continue -and $cleanup.Elapsed.TotalSeconds -lt 15)
    if(-not $receipt.safe_to_continue){$receipt.error=([string]$receipt.error)+'; Excel or capture port remains occupied. Pending cases will not start.'}
    if(-not [IO.File]::Exists((Join-Path $root 'case-metrics.json'))){
        Write-VVJson (Join-Path $root 'case-metrics.json') @{schema='vv-suite-case-metrics-v1';status=$receipt.status;workload_ms=$null;errors=@($receipt.error);workbooks=@();captures=@();invocation_error=$receipt.error;canonical_sheets=@()}
    }
    if(-not [IO.File]::Exists((Join-Path $root 'report.html'))){
        $message=[Net.WebUtility]::HtmlEncode([string]$receipt.error)
        [IO.File]::WriteAllText((Join-Path $root 'report.html'),('<!doctype html><html lang="en"><meta charset="utf-8"><title>Interrupted case</title><body><h1>Case '+$receipt.status+'</h1><p>'+$message+'</p><p><a href="worker.stdout.log">Worker stdout</a> · <a href="worker.stderr.log">Worker stderr</a> · <a href="case-metrics.json">Metrics</a> · <a href="../../report.html">Campaign</a></p><p>Existing captures and progress files have not been deleted.</p></body></html>'),$utf8)
    }
    if($null -ne ('VVSnapshotFile' -as [type])) {
        $receipt.report_io=@{policy=[VVSnapshotFile]::Policy;reader_retries=[VVSnapshotFile]::ReadRetries;publisher_retries=[VVSnapshotFile]::PublishRetries}
    }
    Write-VVJson (Join-Path $root 'supervisor-result.json') $receipt
    $env:VV_DECISION_OWNER_TOKEN=$priorToken;$env:VV_DECISION_CASE_ROOT=$priorCase;$env:VV_DECISION_PREEXISTING_EXCEL=$priorExisting
}
if($receipt.status -ne 'complete'){exit 1}
