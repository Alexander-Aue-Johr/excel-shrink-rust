param(
    [string]$ConfigPath,
    [ValidateSet('recommended','inventory','diagnostics')][string]$Profile,
    [string]$CorpusRoot,
    [switch]$PlanOnly,
    [switch]$NoOpen
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\plan.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\metrics.ps1')
$repo=Get-VVRepo $PSScriptRoot
if([string]::IsNullOrWhiteSpace($ConfigPath)){$ConfigPath=Join-Path $PSScriptRoot 'suite.json'}
$ConfigPath=[IO.Path]::GetFullPath($ConfigPath)
$owner=$null;$state=$null;$jobObject=$null;$process=$null;$exitCode=0;$cfg=$null
function Assert-VVSuiteIdle {
    $excel=@(Get-Process -Name EXCEL -ErrorAction SilentlyContinue)
    if($excel.Count -gt 0){throw ('Excel is already running (PID '+(($excel|ForEach-Object{$_.Id}) -join ', ')+'). Close these instances before continuing. No unrelated process was stopped.')}
    $ports=@([Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners()|Where-Object{$_.Port -in @(3000,3001)})
    if($ports.Count -gt 0){throw 'Port 3000 or 3001 is occupied. The suite will not terminate an unrelated listener.'}
}
function Save-VVSuiteState {
    Write-VVJson (Join-Path $owner.Path 'suite-state.json') $state
    Write-VVSuiteView $owner.Path $state
}
function Test-VVStagedInputs {
    foreach($f in $state.inputs) {
        $p=Join-Path $state.staged_corpus $f.name
        if(-not(Test-Path -LiteralPath $p -PathType Leaf) -or (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant() -ne $f.sha256){throw "Staged input changed: $($f.name). Remaining cases will not run against a different input."}
    }
}
try {
    if(-not $PlanOnly){
        if([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT){throw 'Execute this suite with Windows PowerShell. The VS Code task bridges from WSL automatically.'}
        $owner=Start-VVReport -Start $PSScriptRoot -Experiment 'comparison-suite' -Arguments $PSBoundParameters
        if(-not $owner.Owned){throw 'Start the comparison suite as its own top-level task.'}
    }
    $cfg=Read-VVJson $ConfigPath
    $plan=Get-VVSuitePlan $cfg $Profile
    $selection=Get-VVSuiteInputs $repo $cfg $CorpusRoot
    if($PlanOnly){
        Write-Host "PROFILE: $($plan.profile) | $($plan.jobs.Count) sequential cases | $($selection.files.Count) workbook(s)"
        $plan.jobs|Select-Object sequence,candidate_id,phase,round,position|Format-Table -AutoSize
        Write-Host 'No workbooks opened. No files, certificates or registrations changed.'
        return
    }
    $state=[pscustomobject]@{schema='vv-comparison-result-v1';run_id=(Split-Path $owner.Path -Leaf);profile=$plan.profile;
        description=$plan.description;status='running';started_utc=[DateTime]::UtcNow.ToString('o');config=$cfg;
        catalog=$plan.catalog;jobs=$plan.jobs;ordering=$plan.ordering;inputs=@();errors=@();staged_corpus=(Join-Path $owner.Path 'inputs\corpus');
        comparisons_note='Runtime, bounds/coverage and pixel equivalence are separate outcomes. One machine and one workbook do not establish a universally fastest implementation.'}
    Save-VVSuiteState
    Assert-VVSuiteIdle
    $null=Add-Type -Path (Join-Path $PSScriptRoot '_shared\suite\SuiteRuntime.cs') -ReferencedAssemblies @('System.dll','WindowsBase.dll','PresentationCore.dll')
    [IO.Directory]::CreateDirectory($state.staged_corpus)|Out-Null
    Copy-Item -LiteralPath $ConfigPath -Destination (Join-Path $owner.Path 'inputs\suite-config.json')
    foreach($f in $selection.files) {
        $hash=(Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $target=Join-Path $state.staged_corpus $f.Name
        [IO.File]::Copy($f.FullName,$target,$false)
        if((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash){throw 'Input changed while being staged.'}
        $state.inputs += [pscustomobject]@{name=$f.Name;source=$f.FullName;sha256=$hash;bytes=$f.Length}
    }
    Write-VVJson (Join-Path $owner.Path 'inputs\workbooks.json') $state.inputs
    # Record source versions once. Fixture binaries and past result directories are excluded.
    $sourceRows=@(Get-ChildItem -LiteralPath $PSScriptRoot -File -Recurse|Where-Object{
        $_.Extension -in @('.ps1','.js','.cjs','.cs','.rs','.toml','.py') -and $_.FullName -notmatch '[\\/](results|target|bin|obj)[\\/]'
    }|Sort-Object FullName|ForEach-Object{[pscustomobject]@{path=(Get-VVRelative $repo $_.FullName);sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
    Write-VVJson (Join-Path $owner.Path 'inputs\source-fingerprints.json') $sourceRows
    Save-VVSuiteState
    $worker=Join-Path $PSScriptRoot '_shared\suite\worker.ps1'
    $powershell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    foreach($job in $state.jobs) {
        Assert-VVSuiteIdle
        Test-VVStagedInputs
        $caseRoot=Join-Path $owner.Path ('cases\'+$job.id);[IO.Directory]::CreateDirectory($caseRoot)|Out-Null
        $job.report='cases/'+$job.id+'/report.html';$job.status='running'
        Write-VVJson (Join-Path $caseRoot 'run.json') ([ordered]@{schema='vv-report-v1';id=$job.id;experiment=$job.candidate.label;status='queued';owner_pid=0;
            started_utc=[DateTime]::UtcNow.ToString('o');report_directory=$caseRoot;arguments=@{Suite=$plan.profile;Candidate=$job.candidate_id;Phase=$job.phase;Round=$job.round};
            suite_parent=$state.run_id})
        $spec=[ordered]@{schema='vv-suite-worker-v1';repo_root=$repo;suite_root=$owner.Path;case_root=$caseRoot;job=$job;
            inputs=$state.inputs;staged_corpus=$state.staged_corpus;capture=$cfg.capture;verify_final_pixels=[bool]$cfg.execution.verify_final_pixels;
            parent_pid=$PID;release_token=[Guid]::NewGuid().ToString('N')}
        $specPath=Join-Path $caseRoot 'case-spec.json';Write-VVJson $specPath $spec
        Save-VVSuiteState
        Write-Host ("[{0}/{1}] {2} | {3} {4}" -f $job.sequence,$state.jobs.Count,$job.candidate.label,$job.phase,$job.round)
        # EncodedCommand avoids quoting problems with spaces, apostrophes and WSL UNC paths.
        $command="& '"+$worker.Replace("'","''")+"' -SpecPath '"+$specPath.Replace("'","''")+"'"
        $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
        $timedOut=$false
        try {
            $jobObject=New-Object VVSuiteJob
            $process=Start-Process -NoNewWindow -FilePath $powershell -ArgumentList @('-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-EncodedCommand',$encoded) `
                -WorkingDirectory $env:TEMP -RedirectStandardOutput (Join-Path $caseRoot 'worker.stdout.log') `
                -RedirectStandardError (Join-Path $caseRoot 'worker.stderr.log') -PassThru
            # The worker is still waiting on .go. Failure here cannot start an unowned Excel process.
            try{$jobObject.Assign($process)}catch{try{$process.Kill()}catch{};throw}
            [IO.File]::WriteAllText((Join-Path $caseRoot '.go'),$spec.release_token,[Text.UTF8Encoding]::new($false))
            $clock=[Diagnostics.Stopwatch]::StartNew();$nextLog=15
            while(-not $process.WaitForExit(1000)) {
                if($clock.Elapsed.TotalSeconds -ge [int]$cfg.execution.case_timeout_seconds){$timedOut=$true;$jobObject.Terminate();[void]$process.WaitForExit(10000);break}
                if($clock.Elapsed.TotalSeconds -ge $nextLog) {
                    Write-Host ("  Running {0} | elapsed {1:n0} s | log: {2}" -f $job.candidate_id,$clock.Elapsed.TotalSeconds,(Join-Path $caseRoot 'console.log'))
                    Write-VVEvent 'suite_heartbeat' @{case=$job.id;elapsed_seconds=$clock.Elapsed.TotalSeconds}
                    $nextLog += 15
                }
            }
            $process.Refresh()
            $nativeExit=if($process.HasExited){$process.ExitCode}else{$null}
            $m=Read-VVJson (Join-Path $caseRoot 'case-metrics.json')
            if($timedOut){$job.status='timeout';$job.error='Case safety timeout; owned worker processes were terminated. This is not evidence of an image-size limit.'}
            elseif($null -eq $m){$job.status='failed';$job.error='Worker exited without case-metrics.json. Inspect its stdout/stderr and retained report.'}
            else{$job.status=[string]$m.status;$job.metrics=$m;if($nativeExit -ne 0){$job.status='failed';$job.error=[string]$m.invocation_error}}
        } catch {
            $job.status='failed';$job.error=$_.Exception.Message
            Write-VVEvent 'suite_case_launch_failed' @{case=$job.id;error=$job.error}
        } finally {
            if($null -ne $jobObject){$jobObject.Dispose();$jobObject=$null}
            if($null -ne $process){$process.Dispose();$process=$null}
        }
        if($job.status -in @('timeout','failed') -and -not (Test-VVRunActive $caseRoot)){
            try{Complete-VVReport $caseRoot -Interrupted;Connect-VVCaseReport $caseRoot;Export-VVMetadataBundle $caseRoot}catch{Write-Warning ('Case recovery: '+$_.Exception.Message)}
        }
        Save-VVSuiteState
        Write-Host ("  Result: {0} | {1}" -f $job.status,(Join-Path $caseRoot 'report.html'))
        Test-VVStagedInputs
        Start-Sleep -Seconds ([int]$cfg.execution.cooldown_seconds)
        # A COM/DCOM instance outside the owned Job Object may survive a crash. Never kill arbitrary Excel PIDs.
        $idleClock=[Diagnostics.Stopwatch]::StartNew()
        do {
            try{Assert-VVSuiteIdle;$idle=$true}catch{$idle=$false;$idleError=$_.Exception.Message}
            if(-not $idle){Start-Sleep -Seconds 1}
        }while(-not $idle -and $idleClock.Elapsed.TotalSeconds -lt 10)
        if(-not $idle){throw ('Suite paused after '+$job.id+': '+$idleError+' Pending cases remain recorded; no new Excel session was started.')}
    }
} catch {
    $exitCode=1
    if($null -ne $state){$state.errors+= $_.Exception.Message}
    if($null -ne $owner -and $owner.Owned){try{Write-VVFailure $_ $PSCommandPath}catch{}}
    Write-Warning $_.Exception.Message
} finally {
    if($null -ne $jobObject){$jobObject.Dispose()}
    if($null -ne $process){$process.Dispose()}
    if($null -ne $owner -and $owner.Owned) {
        if($null -ne $state) {
            foreach($j in $state.jobs){if($j.status -eq 'running'){$j.status='interrupted'}elseif($j.status -eq 'queued'){$j.status='not-run'}}
            try{Save-VVSuiteState}catch{Write-Warning ('Could not write final suite state: '+$_.Exception.Message)}
            if(@($state.jobs|Where-Object{$_.phase -eq 'measured' -and $_.status -ne 'complete'}).Count -gt 0){$exitCode=1}
        }
        Stop-VVReport $owner
        if(-not $NoOpen -and ($null -eq $cfg -or $cfg.execution.open_report)) {try{Invoke-Item -LiteralPath (Join-Path $owner.Path 'report.html')}catch{}}
    }
}
if($exitCode -ne 0){exit $exitCode}
