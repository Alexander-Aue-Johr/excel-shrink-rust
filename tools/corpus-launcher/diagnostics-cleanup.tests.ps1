$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '../ExcelDiagnostics.psm1') -Force
$root=Join-Path $PSScriptRoot ('../../.tmp/diagnostic-cleanup-test-'+[Guid]::NewGuid().ToString('N')+'/Diagnostics/EXCEL')
[void][IO.Directory]::CreateDirectory((Join-Path $root 'Additional'))
$old=Join-Path $root 'Additional/old.log'
$fresh=Join-Path $root 'fresh.log'
$keep=Join-Path $root 'keep.xlsx'
$locked=Join-Path $root 'locked.log'
foreach($path in @($old,$fresh,$keep,$locked)){[IO.File]::WriteAllText($path,'test')}
foreach($path in @($old,$locked)){(Get-Item -LiteralPath $path).LastWriteTimeUtc=[DateTime]::UtcNow.AddMinutes(-5)}
$handle=[IO.File]::Open($locked,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
try {
    $result=Clear-ExcelDiagnosticLogs -Root $root
    if((Test-Path -LiteralPath $old) -or $result.removed_files -ne 1){throw 'Old log was not removed'}
    foreach($path in @($fresh,$keep,$locked)){if(-not(Test-Path -LiteralPath $path)){throw 'Protected file was removed'}}
    if($result.skipped_files -ne 1){throw 'Locked file not counted'}
    $rejected=$false
    try { Clear-ExcelDiagnosticLogs -Root (Split-Path $root) } catch { $rejected=$true }
    if(-not $rejected){throw 'Unrestricted root accepted'}
    'PASS: old logs removed; new, locked and non-log files retained; invalid root rejected.'
} finally { $handle.Dispose() }
