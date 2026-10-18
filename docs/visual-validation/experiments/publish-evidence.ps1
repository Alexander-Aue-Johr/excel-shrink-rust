[CmdletBinding()]
param([string]$RunDirectory)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\controller.ps1')
$repo=Get-VVRepo $PSScriptRoot
if(-not $RunDirectory){
 $latest=@(Get-VVRunList (Join-Path $repo 'docs\visual-validation\results')|Where-Object{$_.Meta.experiment -eq 'visual-evaluation' -and -not $_.Active -and $_.Meta.status -ne 'running'}|Select-Object -First 1)
 if($latest.Count -ne 1){throw 'No finished evaluation run is available.'};$RunDirectory=$latest[0].Path
}
$RunDirectory=[IO.Path]::GetFullPath($RunDirectory)
if(Test-VVRunActive $RunDirectory){throw 'The selected run is active.'}
$code=Invoke-VVEvaluationPython $repo @('--publish-run',$RunDirectory)
if($code){throw 'Evidence publication failed; previous published records were not replaced.'}
