param(
    [ValidateSet('Open','Latest','Clean','Recover','Refresh')][string]$Action='Open',
    [ValidateRange(0,10000)][int]$KeepLast=10,
    [switch]$Apply,
    [switch]$ConfirmInGui
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
$repo=Get-VVRepo $PSScriptRoot
$root=Join-Path $repo 'docs\visual-validation\results'
[IO.Directory]::CreateDirectory($root) | Out-Null
if ($Action -eq 'Recover') {
    foreach($run in @(Get-VVRunList $root)) {
        if($run.Meta.status -ne 'running' -or $run.Active){continue}
        Write-Host ('Recovering interrupted report: '+$run.Path)
        Complete-VVReport -RunRoot $run.Path -Interrupted
        Export-VVBundle $run.Path
    }
}
if ($Action -eq 'Clean') {
    $all=@(Get-VVRunList $root)
    $finished=@($all | Where-Object {-not $_.Active -and $_.Meta.status -ne 'running'})
    $remove=@($finished | Select-Object -Skip $KeepLast)
    Write-Host "Keeping the latest $KeepLast finished managed reports; running/unrecognised reports are never removed."
    foreach($run in $remove){Write-Host ('REMOVE: '+$run.Path)}
    if($remove.Count -eq 0){Write-Host 'Nothing to remove.';return}
    if($ConfirmInGui){
        Add-Type -AssemblyName PresentationFramework
        $answer=[Windows.MessageBox]::Show("Delete $($remove.Count) completed reports?`nKeep the latest $KeepLast completed runs.`nInput workbooks, baselines, build caches and unmanaged reports are excluded.", 'Report retention', [Windows.MessageBoxButton]::YesNo, [Windows.MessageBoxImage]::Warning, [Windows.MessageBoxResult]::No)
        $Apply=($answer -eq [Windows.MessageBoxResult]::Yes)
    }
    if(-not $Apply){Write-Host 'Preview only. Repeat with -Apply to delete the listed managed reports.';return}
    $runsPath=[IO.Path]::GetFullPath((Join-Path $root 'runs'))
    foreach($run in $remove){
        $path=[IO.Path]::GetFullPath($run.Path)
        if((Split-Path -Parent $path) -ne $runsPath){throw 'Refusing cleanup outside the managed runs directory.'}
        $entry=Get-Item -LiteralPath $path
        if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Refusing cleanup of a link/reparse point.'}
        if(@(Get-ChildItem -LiteralPath $path -Recurse -Force | Where-Object {$_.Attributes -band [IO.FileAttributes]::ReparsePoint}).Count -gt 0){throw 'Refusing cleanup of a report containing links/reparse points.'}
        $meta=Read-VVJson (Join-Path $path 'run.json')
        if($meta.schema -ne 'vv-report-v1' -or (Test-VVRunActive $path)){throw 'Report identity/activity changed during cleanup.'}
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}
if($Action -eq 'Refresh') {
    foreach($run in @(Get-VVRunList $root)) {
        if($run.Active -or $run.Meta.status -eq 'running'){continue}
        Write-Host ('Rebuilding report view: '+$run.Name)
        try {
            if($run.Meta.experiment -eq 'comparison-suite'){Complete-VVSuiteReport $run.Path}
            else{Write-VVInspector $run.Path}
            Export-VVBundle $run.Path
        }
        catch { Write-Warning ('Report rebuild failed: '+$run.Path+' - '+$_.Exception.Message) }
    }
}
Update-VVIndex $root
if($Action -eq 'Latest'){
    $latest=Read-VVJson (Join-Path $root 'latest.json')
    if($null -ne $latest -and (Test-Path -LiteralPath $latest.report)){
        $runPath=Split-Path -Parent $latest.report
        $runMeta=Read-VVJson (Join-Path $runPath 'run.json')
        $viewAsset=if($runMeta.experiment -eq 'comparison-suite'){'report-assets\suite.js'}else{'report-assets\inspector.js'}
        if(-not (Test-VVRunActive $runPath) -and -not (Test-Path -LiteralPath (Join-Path $runPath $viewAsset))) {
            if($runMeta.experiment -eq 'comparison-suite'){Complete-VVSuiteReport $runPath}
            else{Write-VVInspector $runPath}
            try { Export-VVBundle $runPath } catch { Write-Warning ('The HTML view was rebuilt, but the review bundle could not be refreshed: '+$_.Exception.Message) }
        }
        Start-Process -FilePath $latest.report|Out-Null
    }
    else {Start-Process -FilePath (Join-Path $root 'index.html')|Out-Null}
}elseif($Action -eq 'Open'){Start-Process -FilePath (Join-Path $root 'index.html')|Out-Null}
Write-Host ('REPORTS: '+$root)
