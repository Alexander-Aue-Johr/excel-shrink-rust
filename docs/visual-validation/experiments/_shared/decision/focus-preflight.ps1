param([Parameter(Mandatory=$true)][string]$RepoRoot,[string]$RunRoot)
$ErrorActionPreference='Stop'
$base=Join-Path $RepoRoot 'docs\visual-validation\experiments'
$paths=@('decision-campaign.ps1','gui.ps1','range-image\run.ps1','_shared\suite\worker.ps1',
    '_shared\decision\focus-gui.ps1','_shared\decision\focus-deadlines.ps1','_shared\decision\supervise.ps1',
    '_shared\office-addin\run.ps1','_shared\office-addin\powershell\focus-observer.ps1','tests\focus-observer.tests.ps1','tests\snapshot-publication.tests.ps1',
    '_shared\office-addin\powershell\asset-preflight.ps1')
$checks=[Collections.Generic.List[string]]::new()
foreach($relative in $paths){
    $tokens=$null;$errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $base $relative),[ref]$tokens,[ref]$errors)
    if($errors.Count){throw ('Focused PowerShell parse failed: '+$relative+': '+(($errors|ForEach-Object{$_.Message}) -join '; '))}
    $checks.Add('parse '+$relative)
}
$expected=@{
    'range-image\run.ps1'=@('SheetNames','StartupTimeoutSeconds','CaptureTimeoutSeconds','NativeCallTimeoutSeconds','DiagnosticLevel');
    '_shared\office-addin\run.ps1'=@('StartupTimeoutSeconds','CaptureTimeoutSeconds','FocusDiagnostics')
}
foreach($relative in $expected.Keys){
    $command=Get-Command (Join-Path $base $relative) -ErrorAction Stop
    foreach($name in $expected[$relative]){if(-not $command.Parameters.ContainsKey($name)){throw ('Missing focus parameter '+$name+' in '+$relative)}}
    $checks.Add('parameter contract '+$relative)
}
. (Join-Path $base '_shared\decision\focus-deadlines.ps1')
$p=@{setup_timeout_seconds=600;startup_timeout_seconds=300;capture_timeout_seconds=1800;workload_timeout_seconds=2730}
if($null -ne (Get-VVFocusDeadlineFailure $p 1000 'capture' 700)){throw 'Long capture was incorrectly limited by the old 360-second deadline'}
if((Get-VVFocusDeadlineFailure $p 2401 'capture' 1801).kind -ne 'capture'){throw 'Capture deadline is not enforced'}
if((Get-VVFocusDeadlineFailure $p 400 'startup' 301).kind -ne 'startup'){throw 'Startup deadline is not enforced'}
if((Get-VVFocusDeadlineFailure $p 2731 'capture' 20).kind -ne 'workload'){throw 'Absolute case deadline is not enforced'}
$checks.Add('separate phase deadlines and absolute cap')
$snapshotRoot=if($RunRoot){$RunRoot}else{Join-Path $RepoRoot '.tmp\visual-validation'}
[void][IO.Directory]::CreateDirectory($snapshotRoot)
& (Join-Path $base 'tests\snapshot-publication.tests.ps1') -ProbeRoot $snapshotRoot -ResultPath (Join-Path $snapshotRoot 'snapshot-preflight.json')
$checks.Add('snapshot publication and launch identities on the report filesystem')
& (Join-Path $base 'tests\focus-observer.tests.ps1')
$checks.Add('observer detachment and separate capture outcome')
if($RunRoot){Write-VVJson (Join-Path $RunRoot 'focus-preflight.json') @{schema='vv-focus-preflight-v1';status='complete';checks=@($checks);powershell_version=$PSVersionTable.PSVersion.ToString();timestamp_utc=[DateTime]::UtcNow.ToString('o')}}
Write-Host ('FOCUSED PREFLIGHT: '+$checks.Count+' parser/contract checks passed; no Excel instance was opened.')
