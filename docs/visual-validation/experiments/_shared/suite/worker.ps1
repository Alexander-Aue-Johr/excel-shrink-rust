param([Parameter(Mandatory=$true)][string]$SpecPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$OutputEncoding=[Console]::OutputEncoding
$utf8=[Text.UTF8Encoding]::new($false)
$spec=[IO.File]::ReadAllText($SpecPath,$utf8)|ConvertFrom-Json
$caseRoot=[IO.Path]::GetFullPath($spec.case_root)
$expected=[IO.Path]::GetFullPath((Join-Path $spec.suite_root ('cases\'+$spec.job.id)))
if($caseRoot -ne $expected -or $spec.job.id -notmatch '^\d{3,8}-[a-z0-9-]+$'){throw 'Invalid case specification.'}
$go=Join-Path $caseRoot '.go'
$wait=[Diagnostics.Stopwatch]::StartNew()
# The controller must attach this waiting process to its Job Object before releasing it.
while(-not(Test-Path -LiteralPath $go)) {
    if($wait.Elapsed.TotalSeconds -gt 30){throw 'Controller did not release the isolated worker.'}
    if($null -eq (Get-Process -Id ([int]$spec.parent_pid) -ErrorAction SilentlyContinue)){throw 'Suite controller exited.'}
    Start-Sleep -Milliseconds 100
}
if([IO.File]::ReadAllText($go).Trim() -ne $spec.release_token){throw 'Worker release token mismatch.'}
. (Join-Path $spec.repo_root 'docs\visual-validation\experiments\_shared\reporting.ps1')
. (Join-Path $PSScriptRoot 'metrics.ps1')
$env:VV_REPORT_DIR=$caseRoot
$meta=Read-VVJson (Join-Path $caseRoot 'run.json');$meta.owner_pid=$PID;$meta.status='running'
Write-VVJson (Join-Path $caseRoot 'run.json') $meta
$active=[IO.File]::Open((Join-Path $caseRoot '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
$transcript=$false;$problem=$null;$metrics=$null;$workloadMs=$null;$exitCode=0
try {
    try{Start-Transcript -LiteralPath (Join-Path $caseRoot 'console.log') -Force|Out-Null;$transcript=$true}catch{}
    $exp=Join-Path $spec.repo_root 'docs\visual-validation\experiments'
    $c=$spec.job.candidate;$cfg=$spec.capture;$corpus=[string]$spec.staged_corpus
    Write-VVEvent 'suite_case_started' @{job=$spec.job;input_manifest=$spec.inputs;transport=$(if($c.host_kind -eq 'rust'){'HTTP'}else{'HTTPS / native'})}
    if($spec.decision){Write-VVJson (Join-Path $caseRoot 'case-phase.json') @{phase='workload';timestamp_utc=[DateTime]::UtcNow.ToString('o')}}
    [IO.Directory]::CreateDirectory((Join-Path $caseRoot 'data'))|Out-Null
    $clock=[Diagnostics.Stopwatch]::StartNew()
    try {
        switch([string]$c.kind) {
            'capture' {
                $params=@{CorpusRoot=$corpus;Workbook='all';Strategy=$c.strategy;Implementation=('officejs-'+$c.host_kind);
                    BatchSize=[int]$cfg.batch_size;TileMegapixels=[int]$cfg.tile_megapixels;TilePacking=[string]$cfg.tile_packing;
                    OverflowMode=[string]$c.overflow_mode;OverflowProbeWidthPx=[int]$cfg.overflow_probe_width_px;
                    OverflowProbeHeightPx=[int]$cfg.overflow_probe_height_px;RustExcelLaunchMode=[string]$cfg.excel_launch_mode;ExtendedErrorLogging=$true;NonInteractive=$true}
                if($c.sheet_names){$params.SheetNames=[string[]]@($c.sheet_names)}
                if($c.context_validation){$params.ContextValidation=[string]$c.context_validation}
                if($null -ne $spec.focus_parameters){
                    $p=$spec.focus_parameters
                    $params.StartupTimeoutSeconds=[int]$p.startup_timeout_seconds
                    $params.CaptureTimeoutSeconds=[int]$p.capture_timeout_seconds
                    $params.InactivityTimeoutSeconds=[int]$p.inactivity_timeout_seconds
                    $params.AbsoluteWorkbookTimeoutSeconds=[int]$p.startup_timeout_seconds+[int]$p.capture_timeout_seconds+15
                    $params.NativeCallTimeoutSeconds=[int]$p.native_call_timeout_seconds
                    $params.DiagnosticLevel=[string]$p.diagnostic_level
                    $params.WaitForStart=[bool]$p.wait_for_start
                    $params.UseOfficeDebugJs=[bool]$p.use_office_debug_js
                }
                & (Join-Path $exp 'range-image\run.ps1') @params
            }
            'vba-picture' {& (Join-Path $exp 'range-image\run.ps1') -CorpusRoot $corpus -Workbook all -Strategy A -Implementation vba -NonInteractive}
            'shape-native' {& (Join-Path $exp 'shape-bounds\run-experiment.ps1') -CorpusRoot $corpus}
            'shape-officejs' {& (Join-Path $exp 'shape-bounds\run.ps1') -CorpusRoot $corpus -Implementation ('officejs-'+$c.host_kind) -RustExcelLaunchMode excel-x -NonInteractive}
            'shape-vsto' {
                $dir=Join-Path $caseRoot 'data\shape-bounds';[IO.Directory]::CreateDirectory($dir)|Out-Null
                $list=Join-Path $dir 'files.txt';[IO.File]::WriteAllLines($list,[string[]]@($spec.inputs|ForEach-Object{Join-Path $corpus $_.name}),$utf8)
                & (Join-Path $exp 'shape-bounds\vsto-addin\run-vsto.ps1') -FileListPath $list -OutputPath (Join-Path $dir 'vsto.json') -SkipIfUnavailable
            }
            'diagnostic' {
                if($c.suite -eq 'height-plateau'){& (Join-Path $exp 'range-image-height-limit\run.ps1') -CorpusRoot $corpus -NonInteractive}
                else{& (Join-Path $exp 'range-image-calibration\run.ps1') -Suite $c.suite -NonInteractive}
            }
            {$_ -in @('lab-setup','lab-capture','lab-memory','lab-rows')} {
                . (Join-Path $PSScriptRoot 'native.ps1')
                Invoke-VVDecisionNative $spec
            }
            default {throw 'Unsupported suite case kind.'}
        }
    } finally {
        $clock.Stop();$workloadMs=$clock.Elapsed.TotalMilliseconds
        if($spec.decision){Write-VVJson (Join-Path $caseRoot 'case-phase.json') @{phase='postprocess';workload_ms=$workloadMs;timestamp_utc=[DateTime]::UtcNow.ToString('o')}}
    }
} catch {
    $problem=$_.Exception.Message;$exitCode=1
    try{Write-VVFailure $_ $PSCommandPath}catch{}
} finally {
    try {
        # Decode after timing, not inside the capture measurement. No screenshot file is modified.
        $pixelRequested=[bool]$spec.verify_final_pixels -and $spec.job.phase -eq 'measured' -and $spec.job.candidate.kind -eq 'capture'
        $wantPixels=$pixelRequested
        if($wantPixels) {
            try {
                # Pixel decoding is optional and runs AFTER measurement. WPF is not
                # required; System.Drawing ships with Windows PowerShell/.NET Framework.
                Add-Type -AssemblyName System.Drawing -ErrorAction Stop
                Add-Type -Path (Join-Path $PSScriptRoot 'SuitePixels.cs') -ReferencedAssemblies @('System.dll','System.Drawing.dll') -ErrorAction Stop
            }
            catch {Write-VVEvent 'offline_pixel_decoder_unavailable' @{message=$_.Exception.Message};$wantPixels=$false}
        }
        $metrics=Get-VVCaseMetrics -CaseRoot $caseRoot -Job $spec.job -VerifyPixels:$wantPixels
        if($pixelRequested -and -not $wantPixels){
            $metrics.pixel_verification_requested=$true
            $metrics.pixel_verification_status='decoder-unavailable'
            $metrics.pixel_unverified_captures=@($metrics.captures).Count
        }
        if($spec.decision){
            . (Join-Path $PSScriptRoot 'extras.ps1')
            $metrics=Add-VVDecisionEvidence -Metrics $metrics -CaseRoot $caseRoot -Spec $spec
        }
        if($null -ne $spec.focus_parameters){
            $metrics|Add-Member -NotePropertyName focus_lifecycle -NotePropertyValue (Read-VVJson (Join-Path $caseRoot 'capture-lifecycle.json')) -Force
        }
        $metrics|Add-Member -NotePropertyName workload_ms -NotePropertyValue $workloadMs
        $metrics|Add-Member -NotePropertyName invocation_error -NotePropertyValue $problem
        if($problem -and $metrics.status -ne 'unavailable'){$metrics.status='failed'}
        if($spec.job.candidate.kind -eq 'capture' -and @($metrics.workbooks|Where-Object{$_.kind -eq 'capture'}).Count -ne @($spec.inputs).Count){$metrics.status='partial'}
        Write-VVJson (Join-Path $caseRoot 'case-metrics.json') $metrics
        Write-VVEvent 'suite_case_finished' @{status=$metrics.status;workload_ms=$workloadMs;pixel_verification_ms=$metrics.pixel_verification_ms}
    }catch{
        $exitCode=1;if(-not $problem){$problem=$_.Exception.Message}
        Write-VVJson (Join-Path $caseRoot 'case-metrics.json') @{schema='vv-suite-case-metrics-v1';status='failed';workload_ms=$workloadMs;errors=@($problem);captures=@();workbooks=@();documents=@()}
    }
    if($transcript){try{Stop-Transcript|Out-Null}catch{}}
    if($spec.decision){Write-VVJson (Join-Path $caseRoot 'case-phase.json') @{phase='report';timestamp_utc=[DateTime]::UtcNow.ToString('o')}}
    $publication=[ordered]@{status='ready';timestamp_utc=[DateTime]::UtcNow.ToString('o')}
    try{Complete-VVReport $caseRoot;Connect-VVCaseReport $caseRoot;if(-not $spec.decision){Export-VVMetadataBundle $caseRoot}else{Set-VVBundleStatus $caseRoot 'building' 'Evidence is included in the parent campaign metadata bundle.'}}catch{
        $publication.status='failed';$publication.message=$_.Exception.Message
        [IO.File]::WriteAllText((Join-Path $caseRoot 'case-report-error.txt'),$_.Exception.ToString(),$utf8)
        try{Set-VVBundleStatus $caseRoot 'failed' $publication.message}catch{}
    }
    try{
        $saved=Read-VVJson (Join-Path $caseRoot 'case-metrics.json')
        if($null -ne $saved){$saved|Add-Member -NotePropertyName report_publication -NotePropertyValue $publication -Force;Write-VVJson (Join-Path $caseRoot 'case-metrics.json') $saved}
    }finally{$active.Dispose()}
}
exit $exitCode
