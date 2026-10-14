param([switch]$PlanOnly,[switch]$NewRun,[switch]$ReportOnly,[switch]$NoOpen)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
$repo=Get-VVRepo $PSScriptRoot
$configPath=Join-Path $PSScriptRoot 'decision-campaign.json'
$cfg=Read-VVJson $configPath
$decisionRoot=$PSScriptRoot
$owner=$null;$exitCode=0
function Invoke-DecisionPython([string[]]$Extra) {
    $script=Join-Path $decisionRoot '_shared\decision\campaign.py'
    if($repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$') {
        $distro=$Matches[1];$linuxRepo='/'+$Matches[2].Replace('\','/')
        $args=@('-d',$distro,'--exec','python3',($linuxRepo+'/docs/visual-validation/experiments/_shared/decision/campaign.py'),'--repo-root',$linuxRepo)
        if($null -ne $owner){
            $prefix='\\wsl.localhost\'+$distro+'\'
            if($owner.Path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){$linuxRun='/'+$owner.Path.Substring($prefix.Length).Replace('\','/')}
            else{
                $prefix='\\wsl$\'+$distro+'\'
                if(-not $owner.Path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Campaign report and repository are not in the same WSL distribution'}
                $linuxRun='/'+$owner.Path.Substring($prefix.Length).Replace('\','/')
            }
            $args+=@('--run-root',$linuxRun,'--parent-pid',[string]$PID)
        }
        & wsl.exe @args @Extra | ForEach-Object { Write-Host $_ }
    } else {
        $python=Get-Command py -ErrorAction SilentlyContinue
        $prefix=@()
        if($null -ne $python){$prefix=@('-3')}else{$python=Get-Command python -ErrorAction SilentlyContinue}
        if($null -eq $python){throw 'Python 3.10+ is required for the standard-library campaign controller. No new Python package is needed.'}
        $args=$prefix+@($script,'--repo-root',$repo)
        if($null -ne $owner){$args+=@('--run-root',$owner.Path,'--parent-pid',[string]$PID)}
        & $python.Source @args @Extra | ForEach-Object { Write-Host $_ }
    }
    return $LASTEXITCODE
}
try {
    if($PlanOnly){$exitCode=Invoke-DecisionPython @('--plan');if($exitCode -ne 0){exit $exitCode};return}
    $reports=Join-Path $repo 'docs\visual-validation\results'
    $existing=@()
    if(-not $NewRun -and ($cfg.auto_resume -or $ReportOnly)){
        $existing=@(Get-VVRunList $reports|Where-Object{$_.Meta.experiment -eq 'decision-campaign' -and -not $_.Active -and ($ReportOnly -or $_.Meta.status -in @('running','interrupted','partial'))}|Select-Object -First 1)
    }
    if($existing.Count){
        $path=$existing[0].Path
        $executionLock=[IO.File]::Open((Join-Path $reports '.execution.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try{$lock=[IO.File]::Open((Join-Path $path '.active.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)}catch{$executionLock.Dispose();throw}
        $previous=$env:VV_REPORT_DIR;$env:VV_REPORT_DIR=$path
        $owner=[pscustomobject]@{Owned=$true;Path=$path;Reports=$reports;Previous=$previous;Lock=$lock;ExecutionLock=$executionLock;Transcript=$false}
        $meta=Read-VVJson (Join-Path $path 'run.json');$meta.owner_pid=$PID
        Write-VVJson (Join-Path $path 'run.json') $meta
        Write-Host ('RESUMING REPORT DIRECTORY: '+$path)
    } else {
        if($ReportOnly){throw 'No decision campaign exists to rebuild'}
        $owner=Start-VVReport -Start $PSScriptRoot -Experiment 'decision-campaign' -Arguments $PSBoundParameters
        if(-not $owner.Owned){throw 'Start the decision campaign as its own task'}
    }
    if($owner.Transcript){Stop-Transcript|Out-Null;$owner.Transcript=$false}
    Set-VVBundleStatus $owner.Path 'building' 'The campaign is recording evidence. The metadata bundle will be published after this session.'
    if(-not $ReportOnly){ & (Join-Path $decisionRoot '_shared\decision\preflight.ps1') -RunRoot $owner.Path }
    $extra=@();if($ReportOnly){$extra+= '--report-only'}
    $exitCode=Invoke-DecisionPython $extra
} catch {
    $exitCode=1
    Write-Warning $_.Exception.Message
    if($null -ne $owner){try{Write-VVFailure $_ $PSCommandPath}catch{}}
} finally {
    if($null -ne $owner -and $owner.Owned){
        # Python owns campaign report/bundle publication. Do not overwrite it with the
        # generic single-experiment renderer or repack all images into a second archive.
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
