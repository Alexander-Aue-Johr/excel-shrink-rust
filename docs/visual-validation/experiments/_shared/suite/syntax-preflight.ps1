param([Parameter(Mandatory=$true)][string]$RepoRoot,[string]$RunRoot)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\reporting.ps1')
$base=Join-Path $RepoRoot 'docs\visual-validation\experiments';$checks=[Collections.Generic.List[string]]::new()
foreach($f in @(Get-ChildItem -LiteralPath $base -Recurse -File -Filter '*.ps1'|Where-Object{$_.FullName -notmatch '[\\/]results[\\/]'})){
 $tokens=$null;$errors=$null;[void][Management.Automation.Language.Parser]::ParseFile($f.FullName,[ref]$tokens,[ref]$errors)
 if($errors.Count){throw ('PowerShell parse failed: '+$f.FullName+': '+(($errors|ForEach-Object{$_.Message}) -join '; '))}
}
$checks.Add('PowerShell syntax')
$c=Get-Command (Join-Path $base 'range-image\run.ps1') -ErrorAction Stop
foreach($name in @('SheetNames','StartupTimeoutSeconds','CaptureTimeoutSeconds','NativeCallTimeoutSeconds','DiagnosticLevel')){if(-not$c.Parameters.ContainsKey($name)){throw ('Missing capture parameter '+$name)}}
$checks.Add('Capture parameter contract')
. (Join-Path $PSScriptRoot 'deadlines.ps1')
$p=@{setup_timeout_seconds=600;startup_timeout_seconds=300;capture_timeout_seconds=1800;workload_timeout_seconds=2730}
if($null -ne(Get-VVPhaseDeadlineFailure $p 1000 'capture' 700)){throw 'Capture allowance shortened'}
if((Get-VVPhaseDeadlineFailure $p 2401 'capture' 1801).kind -ne 'capture'){throw 'Capture deadline not enforced'}
if((Get-VVPhaseDeadlineFailure $p 400 'startup' 301).kind -ne 'startup'){throw 'Startup deadline not enforced'}
if((Get-VVPhaseDeadlineFailure $p 2731 'capture' 20).kind -ne 'workload'){throw 'Absolute deadline not enforced'}
$checks.Add('Independent phase limits')
$snapshotRoot=if($RunRoot){$RunRoot}else{Join-Path $RepoRoot '.tmp\visual-validation'};[IO.Directory]::CreateDirectory($snapshotRoot)|Out-Null
& (Join-Path $base 'tests\snapshot-publication.tests.ps1') -ProbeRoot $snapshotRoot -ResultPath (Join-Path $snapshotRoot 'snapshot-preflight.json')
$checks.Add('Snapshot publication on report filesystem')
& (Join-Path $base 'tests\focus-observer.tests.ps1');$checks.Add('Lifecycle observer')
if($RunRoot){Write-VVJson (Join-Path $RunRoot 'syntax-preflight.json') @{schema='vv-runtime-preflight-v1';status='complete';checks=@($checks);powershell_version=$PSVersionTable.PSVersion.ToString()}}
Write-Host ('PREFLIGHT: '+$checks.Count+' runtime checks passed; no Excel instance was opened.')
