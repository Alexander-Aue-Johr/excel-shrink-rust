$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '../ExcelDiagnostics.psm1') -Force
$root=Join-Path $PSScriptRoot ('../../.tmp/diagnostic-cleanup-test-'+[Guid]::NewGuid().ToString('N')+'/Diagnostics/EXCEL')
[void][IO.Directory]::CreateDirectory((Join-Path $root 'Additional'))
$old=Join-Path $root 'Additional/old.log'
$fresh=Join-Path $root 'fresh.log'
$keep=Join-Path $root 'keep.xlsx'
$locked=Join-Path $root 'locked.log'
$live=Join-Path $root 'live.log'
$unknown=Join-Path $root 'unknown.log'
$reused=Join-Path $root 'reused.log'
$ambiguous=Join-Path $root 'ambiguous.log'
$moved=Join-Path $root 'copied.log'
function Log([string]$path, [int]$owner, [DateTime]$timestamp) {
    $line = $timestamp.ToString('MM/dd/yyyy HH:mm:ss.fff', [Globalization.CultureInfo]::InvariantCulture) + "`tEXCEL (0x" + $owner.ToString('X') + ")`t0x1`ttest`n"
    [IO.File]::WriteAllText($path, "Timestamp`tProcess`tTID`tMessage`n" + $line)
    [IO.File]::SetCreationTimeUtc($path, $timestamp)
    [IO.File]::SetLastWriteTimeUtc($path, [DateTime]::UtcNow.AddMinutes(-5))
}
$stamp=[DateTime]::UtcNow.AddMinutes(-5)
$birth=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime()
foreach($path in @($old,$fresh,$locked)){Log $path ([int]::MaxValue) $stamp}
Log $live $PID ([DateTime]::UtcNow)
Log $reused $PID ($birth.AddSeconds(-10))
Log $ambiguous $PID ($birth.AddSeconds(-2))
Log $moved ([int]::MaxValue) $stamp
[IO.File]::SetCreationTimeUtc($moved,[DateTime]::UtcNow)
[IO.File]::SetLastWriteTimeUtc($fresh,[DateTime]::UtcNow)
foreach($path in @($keep,$unknown)){[IO.File]::WriteAllText($path,'unknown owner');[IO.File]::SetLastWriteTimeUtc($path,$stamp)}
# Windows permits Remove-Item against some open files. Cleanup must still keep
# this file: the logger explicitly allows delete sharing, yet remains active.
$handle=[IO.File]::Open($locked,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
try {
    $result=Clear-ExcelDiagnosticLogs -Root $root
    if((Test-Path -LiteralPath $old) -or (Test-Path -LiteralPath $reused) -or $result.removed_files -ne 2){throw ('Confirmed exited owners were not removed: '+($result|ConvertTo-Json -Compress))}
    foreach($path in @($fresh,$keep,$locked,$live,$unknown,$ambiguous,$moved)){if(-not(Test-Path -LiteralPath $path)){throw "Protected file was removed: $path"}}
    if($result.outcomes['locked'] -ne 1 -or $result.retained_locked_bytes -le 0){throw 'Delete-sharing logger was not protected'}
    if($result.outcomes['live'] -ne 2 -or $result.retained_live_bytes -le 0){throw 'Live and time-ambiguous owners were not retained'}
    if($result.outcomes['unknown'] -ne 2 -or $result.retained_unknown_bytes -le 0){throw 'Unknown or copied owner was not retained'}
    if($null -eq $result.free_disk_bytes){throw 'Free disk information missing'}
    $rejected=$false
    try { Clear-ExcelDiagnosticLogs -Root (Split-Path $root) } catch { $rejected=$true }
    if(-not $rejected){throw 'Unrestricted root accepted'}
    'PASS: departed/reused owners removed; live, ambiguous, copied, unknown, delete-sharing and recent logs retained.'
    'PASS: ownership counters and disk space reported; invalid root rejected.'
} finally { $handle.Dispose() }

# Blank preallocated logs cannot identify their writer in a header. Observe the
# live file user once, persist PID + birth and the file's own identity, then
# verify that the exact observation permits cleanup only after that user exits.
$blank=Join-Path $root 'preallocated.log'
$fixturePath=Join-Path (Split-Path (Split-Path $root)) 'hold-log.ps1'
$ownershipPath=Join-Path (Split-Path (Split-Path $root)) 'owners.json'
$quotedBlank=$blank.Replace("'", "''")
@"
`$stream=[IO.File]::Open('$quotedBlank',[IO.FileMode]::Create,[IO.FileAccess]::ReadWrite,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
try { `$stream.SetLength(65536); Start-Sleep -Seconds 120 } finally { `$stream.Dispose() }
"@ | Set-Content -LiteralPath $fixturePath -Encoding UTF8
$fixture=Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',('"'+$fixturePath+'"')) -WindowStyle Hidden -PassThru
try {
    $until=[DateTime]::UtcNow.AddSeconds(10)
    while(-not(Test-Path -LiteralPath $blank) -and [DateTime]::UtcNow -lt $until){Start-Sleep -Milliseconds 50}
    if(-not(Test-Path -LiteralPath $blank)){throw 'Live file fixture did not start'}
    $observed=[ExcelDiagnosticFiles]::ObserveOwners($blank)
    if(-not $observed -or -not @($observed.Owners | Where-Object { $_.Pid -eq $fixture.Id -and $_.StartedTicks -eq $fixture.StartTime.ToUniversalTime().Ticks }).Count){throw 'Restart Manager did not identify the live file owner'}
    $result=Clear-ExcelDiagnosticLogs -Root $root -MinimumAgeSeconds 0 -OwnershipPath $ownershipPath
    if(-not(Test-Path -LiteralPath $blank) -or $result.observed_files -lt 1){throw 'Live blank log was not retained with ownership evidence'}
    $fixture.Kill()
    if(-not $fixture.WaitForExit(10000)){throw 'Log fixture did not exit'}
    $result=Clear-ExcelDiagnosticLogs -Root $root -MinimumAgeSeconds 0 -OwnershipPath $ownershipPath
    if(Test-Path -LiteralPath $blank){throw 'Confirmed departed owner of blank log was not cleaned'}
    # Replacing the path invalidates historical file ownership, even if the
    # replacement is also blank and there are no live holders.
    [IO.File]::WriteAllText($blank,'replacement without owner')
    [IO.File]::SetCreationTimeUtc($blank,[DateTime]::UtcNow.AddSeconds(1))
    $stale=@{path=$blank;identity=$observed.FileIdentity;owners=$observed.Owners}
    ConvertTo-Json -InputObject @($stale) -Depth 4 | Set-Content -LiteralPath $ownershipPath -Encoding UTF8
    $result=Clear-ExcelDiagnosticLogs -Root $root -MinimumAgeSeconds 0 -OwnershipPath $ownershipPath
    if(-not(Test-Path -LiteralPath $blank)){throw 'Replaced file inherited an old owner'}
    'PASS: live Restart Manager ownership persists; blank logs wait for owner exit; replaced files cannot inherit ownership.'
} finally { if(-not $fixture.HasExited){$fixture.Kill()}; $fixture.Dispose() }
