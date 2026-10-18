<#
.SYNOPSIS
Execute the pinned reproduction suite or an explicitly selected individual experiment.
.DESCRIPTION
PlanOnly resolves the complete selection without opening Excel. ResumeRun requires
an unchanged source/input plan. ReportOnly never opens Excel.
#>
[CmdletBinding()]
param(
    [switch]$PlanOnly,[switch]$NoOpen,[switch]$Single,[switch]$ReportOnly,[switch]$FullMetadata,[string]$ResumeRun,
    [string]$ConfigPath,[string]$SelectionFile,[string]$CorpusRoot,[string[]]$Group,
    [string]$Experiment='range-image',[string]$Implementation,[string]$Workbook,
    [Alias('SheetNames')][string[]]$Sheet,[int[]]$SheetIndex,[string]$SheetsJson,
    [switch]$ListSheets,[switch]$ListExperiments,
    [ValidateSet('A','B','C','D','E','F')][string]$Strategy='F',
    [ValidateSet('rust','dotnet')][string]$HostKind='dotnet',
    [ValidateSet('heuristic','selective')][string]$OverflowMode='selective',
    [ValidateSet(16,32,64)][int]$TileMegapixels=32,[ValidateSet(1,2,4,8,16,32,64)][int]$BatchSize=1,
    [ValidateSet('budget','balanced')][string]$TilePacking='budget',
    [ValidateSet('excel-x','rust-com')][string]$ExcelLaunchMode='excel-x',
    [ValidateSet('off','first-seam')][string]$ContextValidation='off',
    [ValidateSet('normal','detailed')][string]$DiagnosticLevel='normal',
    [int]$OverflowProbeWidthPx=1920,[int]$OverflowProbeHeightPx=1080,
    [ValidateRange(30,1800)][int]$SetupTimeoutSeconds=600,[ValidateRange(10,1800)][int]$StartupTimeoutSeconds=300,
    [Alias('TimeoutSeconds')][ValidateRange(30,7200)][int]$CaptureTimeoutSeconds=1200,
    [ValidateRange(10,1800)][int]$InactivityTimeoutSeconds=120,[ValidateRange(10,600)][int]$NativeCallTimeoutSeconds=90,
    [ValidateRange(30,1800)][int]$PostprocessTimeoutSeconds=600,[ValidateRange(60,86400)][int]$SessionTimeoutSeconds=21600,
    [ValidateRange(1,5)][int]$Repetitions=3,[switch]$WaitForStart,[switch]$UseOfficeDebugJs
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\controller.ps1')
$repo=Get-VVRepo $PSScriptRoot
$owner=$null;$exitCode=0;$resolved=$null
try {
    if($ListExperiments){$exitCode=Invoke-VVEvaluationPython $repo @('--list-experiments');if($exitCode){throw 'Cannot load experiments'};return}
    if($FullMetadata -and -not $ReportOnly){throw 'FullMetadata requires ReportOnly; it does not change the test workload.'}
    if($ReportOnly -and -not $ResumeRun){throw 'ReportOnly requires an explicit ResumeRun directory.'}
    if($ResumeRun -and @($PSBoundParameters.Keys|Where-Object{$_ -notin @('ResumeRun','NoOpen','ReportOnly','FullMetadata')}).Count){throw 'Resume uses the saved plan, not new settings.'}
    if($ResumeRun){
        $reports=Join-Path $repo 'docs\visual-validation\results';$path=[IO.Path]::GetFullPath($ResumeRun)
        if((Split-Path -Parent $path) -ine [IO.Path]::GetFullPath((Join-Path $reports 'runs'))){throw 'ResumeRun must be a managed run directory.'}
        if(Test-VVRunActive $path){throw 'Run is active'}
        $resolved=Join-Path $path 'resolved-plan.json'
        if(-not(Test-Path -LiteralPath $resolved)){throw 'No resolved evaluation plan.'}
        $execution=[IO.File]::Open((Join-Path $reports '.execution.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try{$lock=[IO.File]::Open((Join-Path $path '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)}catch{$execution.Dispose();throw}
        $old=$env:VV_REPORT_DIR;$env:VV_REPORT_DIR=$path
        $owner=[pscustomobject]@{Owned=$true;Path=$path;Reports=$reports;Previous=$old;Lock=$lock;ExecutionLock=$execution;Transcript=$false}
        $meta=Read-VVJson (Join-Path $path 'run.json');$meta.owner_pid=$PID;Write-VVJson (Join-Path $path 'run.json') $meta
        Write-Host ('RESUMING REPORT DIRECTORY: '+$path)
    }else{
        $temp=Join-Path $repo ('.tmp\visual-validation\evaluation-launch\'+[Guid]::NewGuid().ToString('N'));[IO.Directory]::CreateDirectory($temp)|Out-Null
        if(-not $ConfigPath){$ConfigPath=Join-Path $PSScriptRoot 'reproduce.json'}
        $cfg=Read-VVJson ([IO.Path]::GetFullPath($ConfigPath));if($null -eq $cfg){throw 'Evaluation config missing'}
        $config=Join-Path $temp 'config.json';Write-VVJson $config $cfg
        $request=@{schema='vv-experiment-request-v1';mode=$(if($Single){'single'}else{'suite'})}
        if($SelectionFile){$request=Convert-VVPlainObject (Read-VVJson ([IO.Path]::GetFullPath($SelectionFile)));if($null -eq $request){throw 'Selection file missing'}}
        if($Single){$request.mode='single';if(-not $request.ContainsKey('repetitions')){$request.repetitions=1}}
        foreach($pair in @(@('CorpusRoot','corpus_root'),@('Workbook','workbook'),@('Sheet','sheets'),@('SheetIndex','sheet_indexes'),@('Group','groups'),@('Repetitions','repetitions'),@('Implementation','implementation'))){
            if($PSBoundParameters.ContainsKey($pair[0])){$request[$pair[1]]=$PSBoundParameters[$pair[0]]}
        }
        if($Single -and ($PSBoundParameters.ContainsKey('Experiment') -or -not $request.ContainsKey('experiment'))){$request.experiment=$Experiment}
        if($SheetsJson){if($Sheet -or $SheetIndex){throw 'Choose one worksheet selector'};$request.sheets=@($SheetsJson|ConvertFrom-Json)}
        $map=@{Strategy='strategy';HostKind='host_kind';OverflowMode='overflow_mode';TileMegapixels='tile_megapixels';BatchSize='batch_size';TilePacking='tile_packing';ExcelLaunchMode='excel_launch_mode';ContextValidation='context_validation';DiagnosticLevel='diagnostic_level';OverflowProbeWidthPx='overflow_probe_width_px';OverflowProbeHeightPx='overflow_probe_height_px';WaitForStart='wait_for_start';UseOfficeDebugJs='use_office_debug_js'}
        foreach($key in $map.Keys){if($PSBoundParameters.ContainsKey($key)){
            if(-not $request.ContainsKey('settings')){$request.settings=@{}}
            $value=$PSBoundParameters[$key];if($value -is [Management.Automation.SwitchParameter]){$value=[bool]$value};$request.settings[$map[$key]]=$value
        }}
        $map=@{SetupTimeoutSeconds='setup_timeout_seconds';StartupTimeoutSeconds='startup_timeout_seconds';CaptureTimeoutSeconds='capture_timeout_seconds';InactivityTimeoutSeconds='inactivity_timeout_seconds';NativeCallTimeoutSeconds='native_call_timeout_seconds';PostprocessTimeoutSeconds='postprocess_timeout_seconds';SessionTimeoutSeconds='session_timeout_seconds'}
        foreach($key in $map.Keys){if($PSBoundParameters.ContainsKey($key)){if(-not $request.ContainsKey('limits')){$request.limits=@{}};$request.limits[$map[$key]]=$PSBoundParameters[$key]}}
        $requestPath=Join-Path $temp 'request.json';$resolved=Join-Path $temp 'resolved-plan.json';Write-VVJson $requestPath $request
        $extra=@('--config',$config,'--request',$requestPath,'--write-resolved',$resolved)
        if($ListSheets){$extra+='--list-sheets'}else{$extra+='--plan'}
        $exitCode=Invoke-VVEvaluationPython $repo $extra
        if($exitCode){throw 'Selection or pinned-input validation failed; Excel was not opened.'}
        if($PlanOnly -or $ListSheets){return}
        $owner=Start-VVReport -Start $PSScriptRoot -Experiment 'visual-evaluation' -Arguments $PSBoundParameters
        if(-not $owner.Owned){throw 'Run evaluation as its own task'}
        Write-VVJson (Join-Path $owner.Path 'request.json') $request
        $saved=Join-Path $owner.Path 'resolved-plan.json';[IO.File]::Copy($resolved,$saved,$false);$resolved=$saved
    }
    if($owner.Transcript){Stop-Transcript|Out-Null;$owner.Transcript=$false}
    Set-VVBundleStatus $owner.Path 'building' 'Recording the declared cases.'
    if(-not $ReportOnly){
        & (Join-Path $PSScriptRoot '_shared\suite\syntax-preflight.ps1') -RepoRoot $repo -RunRoot $owner.Path
        & (Join-Path $PSScriptRoot '_shared\suite\preflight.ps1') -RunRoot $owner.Path
    }
    $extra=@('--resolved',$resolved,'--run-root',$owner.Path,'--parent-pid',[string]$PID);if($ReportOnly){$extra+='--report-only'};if($FullMetadata){$extra+='--full-metadata'}
    $exitCode=Invoke-VVEvaluationPython $repo $extra
}catch{
    $exitCode=1;Write-Warning $_.Exception.Message
    if($owner){try{Write-VVFailure $_ $PSCommandPath}catch{}}
}finally{
    if($owner -and $owner.Owned){
        try{
            if(-not(Test-Path -LiteralPath (Join-Path $owner.Path 'evaluation-state.json'))){
                $m=Read-VVJson (Join-Path $owner.Path 'run.json');$m.status='failed';Write-VVJson (Join-Path $owner.Path 'run.json') $m
                Complete-VVReport $owner.Path;Export-VVMetadataBundle $owner.Path
            }
            Update-VVIndex $owner.Reports
        }catch{Write-Warning ('Report finalization: '+$_.Exception.Message)}
        $owner.Lock.Dispose();$owner.ExecutionLock.Dispose();$env:VV_REPORT_DIR=$owner.Previous
        Write-Host ('REPORT DIRECTORY: '+$owner.Path);Write-Host ('REPORT: '+(Join-Path $owner.Path 'report.html'))
        try{$b=Read-VVJson (Join-Path $owner.Path 'bundle-status.json');if($b.status -eq 'ready'){Write-Host ('REVIEW BUNDLE (NO IMAGES): '+(Join-Path $owner.Path 'review-metadata.zip'))}else{Write-Warning 'Bundle not published; evidence remains in the report directory.'}}catch{Write-Warning $_.Exception.Message}
        if(-not $NoOpen){try{Invoke-Item -LiteralPath (Join-Path $owner.Path 'report.html')}catch{}}
    }
}
if($exitCode){exit $exitCode}
