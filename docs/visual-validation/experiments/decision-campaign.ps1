<#
.SYNOPSIS
Run the bounded strategy campaign or an exact selected-sheet reproduction.
.DESCRIPTION
-Focused exposes the same selection and phase deadlines through CLI and WPF.
Use -ListCases, -Workbook <exact filename> -ListSheets, or -PlanOnly without Excel.
Focused runs create a new report by default; -ResumeRun explicitly resumes a compatible
partial run. All array-valued selections are serialized to JSON before WSL/native calls.
#>
[CmdletBinding()]
param(
    [switch]$PlanOnly,[switch]$NewRun,[switch]$ReportOnly,[switch]$NoOpen,
    [switch]$Focused,[switch]$Finalize,[switch]$Campaign,
    [Alias('Profile')][ValidateSet('fixed-width','tile-budget','hosts','overflow','startup','coverage-gaps')][string]$QuestionProfile,
    [ValidateSet('cli','gui')][string]$InterfaceMode='cli',
    [string]$ConfigPath,[string]$SelectionFile,[string]$ResumeRun,
    [string[]]$CaseId,[string]$Workbook,[Alias('SheetNames')][string[]]$Sheet,
    [int[]]$SheetIndex,[string]$CorpusRoot,[switch]$ListCases,[switch]$ListSheets,
    [ValidateSet('rust','dotnet')][string]$HostKind='dotnet',
    [ValidateSet('heuristic','selective')][string]$OverflowMode='selective',
    [ValidateSet(16,32,64)][int]$TileMegapixels=16,
    [ValidateSet('off','first-seam')][string]$ContextValidation='off',
    [ValidateSet('normal','detailed')][string]$DiagnosticLevel='detailed',
    [ValidateRange(30,1800)][int]$SetupTimeoutSeconds=600,
    [ValidateRange(10,1800)][int]$StartupTimeoutSeconds=300,
    [Alias('TimeoutSeconds')][ValidateRange(30,7200)][int]$CaptureTimeoutSeconds=1200,
    [ValidateRange(10,1800)][int]$InactivityTimeoutSeconds=120,
    [ValidateRange(10,600)][int]$NativeCallTimeoutSeconds=90,
    [ValidateRange(30,1800)][int]$PostprocessTimeoutSeconds=240,
    [ValidateRange(60,86400)][int]$SessionTimeoutSeconds=3600,
    [ValidateRange(1,5)][int]$Repetitions=1,
    [switch]$WaitForStart,[switch]$UseOfficeDebugJs
)
$ErrorActionPreference='Stop'
if(@(@($Focused,$Finalize,$Campaign) | Where-Object { [bool]$_ }).Count -gt 1){throw 'Choose exactly one mode: -Focused, -Finalize or -Campaign.'}
if(-not $Focused -and -not $Finalize -and -not $Campaign){
    if($PSBoundParameters.Count){throw 'Select -Focused, -Finalize or -Campaign explicitly. No experiment was started.'}
    Write-Host 'Visual validation — choose an explicit scope'
    Write-Host '  -Focused -InterfaceMode gui        Select a workbook, sheets and deadlines'
    Write-Host '  -Focused -SelectionFile <json>      Replay an exact selection'
    Write-Host '  -Finalize                          Readiness-gated strategy qualification'
    Write-Host '  -Campaign                          Historical broad campaign (may take hours)'
    Write-Host 'No Excel instance, controller or experiment was started.'
    return
}
if($NewRun -and $ResumeRun){throw '-NewRun and -ResumeRun are mutually exclusive.'}
$focusedKeys=@('QuestionProfile','InterfaceMode','SelectionFile','CaseId','Workbook','Sheet','SheetIndex','CorpusRoot','ListCases','ListSheets',
    'HostKind','OverflowMode','TileMegapixels','ContextValidation','DiagnosticLevel','SetupTimeoutSeconds',
    'StartupTimeoutSeconds','CaptureTimeoutSeconds','InactivityTimeoutSeconds','NativeCallTimeoutSeconds',
    'PostprocessTimeoutSeconds','SessionTimeoutSeconds','Repetitions','WaitForStart','UseOfficeDebugJs')
if(-not $Focused){foreach($key in $focusedKeys){if($PSBoundParameters.ContainsKey($key)){throw ('-'+$key+' requires -Focused. Qualification scope will not be narrowed silently.')}}}
if($ListCases -and $ListSheets){throw 'Choose -ListCases or -ListSheets.'}
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
$repo=Get-VVRepo $PSScriptRoot
$experimentName=if($Finalize){'capture-finalization'}elseif($Focused){'capture-followup'}else{'decision-campaign'}
if(-not $ConfigPath){$ConfigPath=Join-Path $PSScriptRoot $(if($Finalize){'capture-finalize.json'}elseif($Focused){'capture-followup.json'}else{'decision-campaign.json'})}
$ConfigPath=[IO.Path]::GetFullPath($ConfigPath)
$cfg=Read-VVJson $ConfigPath
if($null -eq $cfg){throw ('Missing config: '+$ConfigPath)}
$decisionRoot=$PSScriptRoot
$pythonConfigPath=$ConfigPath
$owner=$null;$exitCode=0;$focusRequest=$null;$focusTemp=$null

function Convert-VVControllerPath([string]$Path) {
    $full=[IO.Path]::GetFullPath($Path)
    if($repo -notmatch '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$'){return $full}
    $expectedDistro=$Matches[1]
    if($full -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$') {
        if($Matches[1] -cne $expectedDistro){throw 'Controller files must be in the repository WSL distribution.'}
        return '/'+$Matches[2].Replace('\','/')
    }
    throw 'WSL controller files must be staged under the repository, not an unrelated UNC path.'
}
function Invoke-DecisionPython([string[]]$Extra) {
    $extraArgs=@($Extra)
    if($Focused){$extraArgs+= '--focused'}
    if($Finalize){$extraArgs+= '--finalize'}
    $script=Join-Path $decisionRoot '_shared\decision\campaign.py'
    if($repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$') {
        $distro=$Matches[1];$linuxRepo='/'+$Matches[2].Replace('\','/')
        $nativeArgs=@('-d',$distro,'--cd',$linuxRepo,'--exec','python3','-u',
            ($linuxRepo+'/docs/visual-validation/experiments/_shared/decision/campaign.py'),'--repo-root',$linuxRepo,
            '--config',(Convert-VVControllerPath $pythonConfigPath))
        if($null -ne $owner){$nativeArgs+=@('--run-root',(Convert-VVControllerPath $owner.Path),'--parent-pid',[string]$PID)}
        & wsl.exe @nativeArgs @extraArgs | ForEach-Object { Write-Host $_ }
        $code=$LASTEXITCODE
    } else {
        $python=Get-Command py -ErrorAction SilentlyContinue;$prefix=@()
        if($null -ne $python){$prefix=@('-3')}else{$python=Get-Command python -ErrorAction SilentlyContinue}
        if($null -eq $python){throw 'Python 3.10+ is required for the standard-library controller.'}
        $nativeArgs=$prefix+@('-u',$script,'--repo-root',$repo,'--config',$pythonConfigPath)
        if($null -ne $owner){$nativeArgs+=@('--run-root',$owner.Path,'--parent-pid',[string]$PID)}
        & $python.Source @nativeArgs @extraArgs | ForEach-Object { Write-Host $_ }
        $code=$LASTEXITCODE
    }
    return [int]$code
}
function Convert-VVPlainObject($Object) {
    if($null -eq $Object){return $null}
    if($Object -is [string] -or $Object -is [ValueType]){return $Object}
    if($Object -is [Collections.IDictionary]){$h=@{};foreach($k in $Object.Keys){$h[$k]=Convert-VVPlainObject $Object[$k]};return $h}
    if($Object -is [pscustomobject]){$h=@{};foreach($p in $Object.PSObject.Properties){$h[$p.Name]=Convert-VVPlainObject $p.Value};return $h}
    if($Object -is [array]){return ,@($Object|ForEach-Object{Convert-VVPlainObject $_})}
    return $Object
}
try {
    $resumeFocusConfig=$null
    if($Focused -and $ResumeRun -and -not $ReportOnly) {
        $stored=Join-Path ([IO.Path]::GetFullPath($ResumeRun)) 'focus-resolved.json'
        $overrides=@($focusedKeys|Where-Object{$PSBoundParameters.ContainsKey($_) -and $_ -ne 'InterfaceMode'})
        if($overrides.Count -eq 0 -and (Test-Path -LiteralPath $stored)){
            $resumeFocusConfig=$stored;$pythonConfigPath=$stored;$cfg=Read-VVJson $stored
            $focusRequest=Convert-VVPlainObject (Read-VVJson (Join-Path ([IO.Path]::GetFullPath($ResumeRun)) 'focus-request.json'))
        }
    }
    if($Focused -and -not $ReportOnly -and -not $resumeFocusConfig) {
        $focusTemp=Join-Path $repo ('.tmp\visual-validation\focus-launch\'+[Guid]::NewGuid().ToString('N'))
        [IO.Directory]::CreateDirectory($focusTemp)|Out-Null
        # Stage config too: native invocations pass file paths, not a serialized array argument.
        $baseConfig=Join-Path $focusTemp 'base-config.json';Write-VVJson $baseConfig $cfg;$pythonConfigPath=$baseConfig
        if($ListCases){$exitCode=Invoke-DecisionPython @('--list-focus-cases');if($exitCode -ne 0){exit $exitCode};return}
        $focusRequest=@{schema='vv-focus-request-v1'}
        if($SelectionFile){$loaded=Read-VVJson ([IO.Path]::GetFullPath($SelectionFile));if($null -eq $loaded){throw 'Selection file not found.'};$focusRequest=Convert-VVPlainObject $loaded}
        $map=@{QuestionProfile='profile';CaseId='case_ids';Workbook='workbook';Sheet='sheets';SheetIndex='sheet_indexes';CorpusRoot='corpus_root';
            HostKind='host_kind';OverflowMode='overflow_mode';TileMegapixels='tile_megapixels';ContextValidation='context_validation';
            DiagnosticLevel='diagnostic_level';SetupTimeoutSeconds='setup_timeout_seconds';StartupTimeoutSeconds='startup_timeout_seconds';
            CaptureTimeoutSeconds='capture_timeout_seconds';InactivityTimeoutSeconds='inactivity_timeout_seconds';NativeCallTimeoutSeconds='native_call_timeout_seconds';
            PostprocessTimeoutSeconds='postprocess_timeout_seconds';SessionTimeoutSeconds='session_timeout_seconds';Repetitions='repetitions';
            WaitForStart='wait_for_start';UseOfficeDebugJs='use_office_debug_js'}
        if($PSBoundParameters.ContainsKey('QuestionProfile')){foreach($key in @('case_ids','workbook','targets','sheets','sheet_indexes')){[void]$focusRequest.Remove($key)}}
        if($PSBoundParameters.ContainsKey('Workbook')){[void]$focusRequest.Remove('case_ids');[void]$focusRequest.Remove('targets');[void]$focusRequest.Remove('sheets');[void]$focusRequest.Remove('sheet_indexes')}
        if($PSBoundParameters.ContainsKey('CaseId')){[void]$focusRequest.Remove('workbook');[void]$focusRequest.Remove('targets');[void]$focusRequest.Remove('sheets');[void]$focusRequest.Remove('sheet_indexes')}
        if($PSBoundParameters.ContainsKey('Sheet')){[void]$focusRequest.Remove('sheet_indexes')}
        if($PSBoundParameters.ContainsKey('SheetIndex')){[void]$focusRequest.Remove('sheets')}
        foreach($key in $map.Keys){if($PSBoundParameters.ContainsKey($key)){
            $value=$PSBoundParameters[$key];if($value -is [Management.Automation.SwitchParameter]){$value=[bool]$value}
            $focusRequest[$map[$key]]=$value
        }}
        if($InterfaceMode -eq 'gui'){
            . (Join-Path $PSScriptRoot '_shared\decision\focus-gui.ps1')
            $focusRequest=Show-VVFocusDialog -RepoRoot $repo -InitialRequest $focusRequest
            if($null -eq $focusRequest){Write-Host 'Focused selection cancelled.';return}
        }
        if($ListSheets){if($focusRequest.profile){throw '-ListSheets needs a workbook/case, not a question profile.'};[void]$focusRequest.Remove('sheets');[void]$focusRequest.Remove('sheet_indexes')}
        $requestPath=Join-Path $focusTemp 'request.json';Write-VVJson $requestPath $focusRequest
        $resolvedPath=Join-Path $focusTemp 'effective-config.json'
        $extra=@('--focus-request',(Convert-VVControllerPath $requestPath))
        if($ListSheets){$extra+= '--list-focus-sheets'}else{$extra+=@('--plan','--write-resolved-config',(Convert-VVControllerPath $resolvedPath))}
        $exitCode=Invoke-DecisionPython $extra
        if($exitCode -ne 0){throw ('Focused selection validation failed (exit '+$exitCode+'). No Excel was started. Request: '+$requestPath)}
        if($ListSheets -or $PlanOnly){return}
        $pythonConfigPath=$resolvedPath;$cfg=Read-VVJson $resolvedPath
    } elseif($PlanOnly){$exitCode=Invoke-DecisionPython @('--plan');if($exitCode -ne 0){exit $exitCode};return}
    $reports=Join-Path $repo 'docs\visual-validation\results'
    $existing=@()
    if($ResumeRun){
        $path=[IO.Path]::GetFullPath($ResumeRun)
        $prefix=([IO.Path]::GetFullPath((Join-Path $reports 'runs'))).TrimEnd('\')+'\'
        if(-not $path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw '-ResumeRun must point to this repository managed run directory.'}
        $existing=@(Get-VVRunList $reports|Where-Object{$_.Path -eq $path -and $_.Meta.experiment -eq $experimentName -and -not $_.Active})
        if($existing.Count -ne 1){throw 'Resume directory is missing, active, or belongs to another run type.'}
    } elseif(-not $NewRun -and ($ReportOnly -or (-not $Focused -and $cfg.auto_resume))){
        $existing=@(Get-VVRunList $reports|Where-Object{$_.Meta.experiment -eq $experimentName -and -not $_.Active -and ($ReportOnly -or $_.Meta.status -in @('running','interrupted','partial'))}|Select-Object -First 1)
    }
    if($existing.Count){
        $path=$existing[0].Path
        $executionLock=[IO.File]::Open((Join-Path $reports '.execution.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try{$lock=[IO.File]::Open((Join-Path $path '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)}catch{$executionLock.Dispose();throw}
        $previous=$env:VV_REPORT_DIR;$env:VV_REPORT_DIR=$path
        $owner=[pscustomobject]@{Owned=$true;Path=$path;Reports=$reports;Previous=$previous;Lock=$lock;ExecutionLock=$executionLock;Transcript=$false}
        $meta=Read-VVJson (Join-Path $path 'run.json');$meta.owner_pid=$PID;Write-VVJson (Join-Path $path 'run.json') $meta
        Write-Host ('RESUMING REPORT DIRECTORY: '+$path)
        if($Focused -and $ReportOnly -and (Test-Path -LiteralPath (Join-Path $path 'focus-resolved.json'))){$pythonConfigPath=Join-Path $path 'focus-resolved.json';$cfg=Read-VVJson $pythonConfigPath}
    } else {
        if($ReportOnly){throw 'No campaign exists to rebuild'}
        $owner=Start-VVReport -Start $PSScriptRoot -Experiment $experimentName -Arguments $PSBoundParameters
        if(-not $owner.Owned){throw 'Start this run as its own task'}
    }
    if($owner.Transcript){Stop-Transcript|Out-Null;$owner.Transcript=$false}
    if($Focused -and -not $ReportOnly){Write-VVJson (Join-Path $owner.Path 'focus-launch-request.json') $focusRequest;Write-VVJson (Join-Path $owner.Path 'effective-config.json') $cfg}
    Set-VVBundleStatus $owner.Path 'building' 'The run is recording evidence; a verified metadata bundle will be published after this session.'
    if(-not $ReportOnly){
        if($Focused){ & (Join-Path $decisionRoot '_shared\decision\focus-preflight.ps1') -RepoRoot $repo -RunRoot $owner.Path }
        & (Join-Path $decisionRoot '_shared\decision\preflight.ps1') -RunRoot $owner.Path
    }
    $extra=@();if($ReportOnly){$extra+= '--report-only'}
    $exitCode=Invoke-DecisionPython $extra
} catch {
    $exitCode=1;Write-Warning $_.Exception.Message
    if($null -ne $owner){try{Write-VVFailure $_ $PSCommandPath}catch{}}
} finally {
    if($null -ne $owner -and $owner.Owned){
        # A controller/preflight failure before campaign-state creation still gets
        # the existing Windows-side metadata report; never fabricate a campaign result.
        if(-not (Test-Path -LiteralPath (Join-Path $owner.Path 'campaign-state.json'))){
            try {
                $failedMeta=Read-VVJson (Join-Path $owner.Path 'run.json')
                if($null -ne $failedMeta){$failedMeta.status='failed';Write-VVJson (Join-Path $owner.Path 'run.json') $failedMeta}
                Complete-VVReport $owner.Path
                Export-VVMetadataBundle $owner.Path
            } catch {Write-Warning ('Startup report recovery: '+$_.Exception.Message)}
        }
        try{
            $meta=Read-VVJson (Join-Path $owner.Path 'run.json')
            if($null -ne $meta -and $meta.status -eq 'running'){$meta.status='interrupted';Write-VVJson (Join-Path $owner.Path 'run.json') $meta}
            Update-VVIndex $owner.Reports
        }catch{Write-Warning ('History update: '+$_.Exception.Message)}
        $owner.Lock.Dispose();$owner.ExecutionLock.Dispose();$env:VV_REPORT_DIR=$owner.Previous
        Write-Host ('REPORT DIRECTORY: '+$owner.Path)
        Write-Host ('REPORT: '+(Join-Path $owner.Path 'report.html'))
        $bundle=Read-VVJson (Join-Path $owner.Path 'bundle-status.json')
        if($bundle.status -eq 'ready'){Write-Host ('REVIEW BUNDLE (NO IMAGES): '+(Join-Path $owner.Path 'review-metadata.zip'))}
        else{Write-Host 'Metadata bundle not published. Existing evidence remains in the report directory.'}
        if(-not $NoOpen -and $cfg.open_report){try{Invoke-Item -LiteralPath (Join-Path $owner.Path 'report.html')}catch{}}
    }
}
if($exitCode -ne 0){exit $exitCode}
