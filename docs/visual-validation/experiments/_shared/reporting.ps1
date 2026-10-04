. (Join-Path $PSScriptRoot 'report\inspector.ps1')
# Shared report ownership for ALL visual-validation experiment entry points.
# Reports are durable from the beginning; caches and input workbooks are never cleanup targets.
function Write-VVJson([string]$Path, $Value) {
    $parent = Split-Path -Parent $Path
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $tmp = $Path + '.new'
    [IO.File]::WriteAllText($tmp, ($Value | ConvertTo-Json -Depth 100) + "`n", [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($Path)) {
        try { [IO.File]::Replace($tmp, $Path, $null) }
        catch {
            # Some network/WSL shares do not support ReplaceFile. Keep the previous
            # complete version if a copy-over fallback is required on that filesystem.
            if (-not [IO.File]::Exists($tmp)) { throw }
            [IO.File]::Copy($Path, $Path + '.previous', $true)
            [IO.File]::Copy($tmp, $Path, $true)
            [IO.File]::Delete($tmp)
        }
    } else { [IO.File]::Move($tmp, $Path) }
}
function Read-VVJson([string]$Path) {
    if ([IO.File]::Exists($Path)) { return ([IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) | ConvertFrom-Json) }
    return $null
}
function Get-VVRepo([string]$Start) {
    $here = [IO.Path]::GetFullPath($Start)
    while (-not [string]::IsNullOrWhiteSpace($here)) {
        if (Test-Path -LiteralPath (Join-Path $here 'docs\visual-validation\experiments\run.ps1')) { return $here }
        $next = Split-Path -Parent $here
        if ($next -eq $here) { break }; $here = $next
    }
    throw "Cannot resolve Excel Shrink repository from $Start"
}
function Write-VVEvent([string]$Stage, $Detail = $null) {
    if ([string]::IsNullOrWhiteSpace($env:VV_REPORT_DIR)) { return }
    $record = [ordered]@{ timestamp_utc=[DateTime]::UtcNow.ToString('o'); source='powershell'; stage=$Stage; detail=$Detail }
    [IO.File]::AppendAllText((Join-Path $env:VV_REPORT_DIR 'events.jsonl'), ($record | ConvertTo-Json -Depth 30 -Compress)+"`n", [Text.UTF8Encoding]::new($false))
}
function Test-VVRunActive([string]$Path) {
    try {
        $stream = [IO.File]::Open((Join-Path $Path '.active.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $stream.Dispose(); return $false
    } catch { return $true }
}
function Get-VVRunList([string]$ReportRoot) {
    $runs = Join-Path $ReportRoot 'runs'
    if (-not (Test-Path -LiteralPath $runs)) { return @() }
    return @(Get-ChildItem -LiteralPath $runs -Directory | Where-Object {
        -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    } | ForEach-Object {
        try {
            $meta=Read-VVJson (Join-Path $_.FullName 'run.json')
            if ($null -ne $meta -and $meta.schema -eq 'vv-report-v1') {
                [pscustomobject]@{ Path=$_.FullName; Name=$_.Name; Meta=$meta; Active=(Test-VVRunActive $_.FullName) }
            }
        } catch { Write-Warning ('Unreadable report: '+$_.FullName) }
    } | Sort-Object Name -Descending)
}
function Update-VVIndex([string]$ReportRoot) {
    [IO.Directory]::CreateDirectory($ReportRoot) | Out-Null
    $cards = [Text.StringBuilder]::new()
    $items = @(Get-VVRunList $ReportRoot)
    foreach ($item in $items) {
        $h = [Net.WebUtility]::HtmlEncode([string]$item.Meta.experiment)
        $status = [Net.WebUtility]::HtmlEncode([string]$item.Meta.status)
        if ($item.Meta.status -eq 'running' -and -not $item.Active) { $status='interrupted (report can be recovered)' }
        [void]$cards.AppendLine("<tr><td>$($item.Name)</td><td>$h</td><td>$status</td><td><a href='runs/$($item.Name)/report.html'>Report</a> | <a href='runs/$($item.Name)/review.zip'>ZIP</a></td></tr>")
    }
    $html=@"
<!doctype html><html lang="en"><meta charset="utf-8"><title>Visual validation - Run history</title>
<style>body{font:16px system-ui;max-width:1200px;margin:40px auto;padding:0 20px}table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:12px;border-bottom:1px solid #ccc}a{color:#145e9b}p{line-height:1.6}</style>
<h1>Visual validation - Run history</h1><p>Capture results, diagnostics and review bundles. Most recent runs appear first.</p>
<p>Retention applies to completed managed runs only. Input workbooks, baselines and build caches are excluded.</p>
<table><thead><tr><th>Run (UTC)</th><th>Experiment</th><th>Status</th><th>Artifacts</th></tr></thead><tbody>$cards</tbody></table></html>
"@
    [IO.File]::WriteAllText((Join-Path $ReportRoot 'index.html'),$html,[Text.UTF8Encoding]::new($false))
}
function Start-VVReport([string]$Start, [string]$Experiment, $Arguments) {
    if (-not [string]::IsNullOrWhiteSpace($env:VV_REPORT_DIR)) {
        $existing=Read-VVJson (Join-Path $env:VV_REPORT_DIR 'run.json')
        if ($null -ne $existing -and $existing.schema -eq 'vv-report-v1' -and $existing.owner_pid -eq $PID) {
            Write-VVEvent 'entry_started' @{experiment=$Experiment;arguments=$Arguments}
            return [pscustomobject]@{ Owned=$false }
        }
    }
    $repo=Get-VVRepo $Start
    $reports=Join-Path $repo 'docs\visual-validation\results'
    [IO.Directory]::CreateDirectory($reports) | Out-Null
    try { $executionLock=[IO.File]::Open((Join-Path $reports '.execution.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None) }
    catch { throw 'Another visual-validation experiment is still running. It will not be stopped or overwritten.' }
    $id=[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
    $path=Join-Path $reports ('runs\'+$id)
    [IO.Directory]::CreateDirectory($path) | Out-Null
    $lock=[IO.File]::Open((Join-Path $path '.active.lock'),[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $previous=$env:VV_REPORT_DIR; $env:VV_REPORT_DIR=$path
    $meta=[ordered]@{schema='vv-report-v1';id=$id;experiment=$Experiment;status='running';owner_pid=$PID;
        started_utc=[DateTime]::UtcNow.ToString('o');arguments=$Arguments;report_directory=$path}
    Write-VVJson (Join-Path $path 'run.json') $meta
    Write-VVJson (Join-Path $path 'result.json') @{schema='vv-run-result-v1';status='running';run=$meta;outputs=@();errors=@()}
    [IO.File]::WriteAllText((Join-Path $path 'summary.txt'),"RUNNING: $Experiment`nResults are persisted here even when Excel fails.`n",[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $path 'report.html'),'<meta charset="utf-8"><h1>Capture in progress</h1><p>Results and diagnostic logs are being recorded. The report is finalized when the run completes or is recovered.</p>',[Text.UTF8Encoding]::new($false))
    Write-VVJson (Join-Path $reports 'latest.json') @{id=$id;directory=$path;report=(Join-Path $path 'report.html');zip=(Join-Path $path 'review.zip')}
    $transcript=$false
    try { Start-Transcript -LiteralPath (Join-Path $path 'console.log') -Force | Out-Null; $transcript=$true } catch { Write-VVEvent 'transcript_unavailable' @{message=$_.Exception.Message} }
    Write-VVEvent 'run_started' $meta
    Update-VVIndex $reports
    Write-Host "REPORT DIRECTORY: $path"
    return [pscustomobject]@{Owned=$true;Path=$path;Reports=$reports;Previous=$previous;Lock=$lock;ExecutionLock=$executionLock;Transcript=$transcript}
}
function Write-VVFailure($ErrorRecord, [string]$Entry) {
    if ([string]::IsNullOrWhiteSpace($env:VV_REPORT_DIR)) { return }
    $detail=[ordered]@{entry=$Entry;message=$ErrorRecord.Exception.Message;exception=$ErrorRecord.Exception.ToString();
        stack=$ErrorRecord.ScriptStackTrace;position=$ErrorRecord.InvocationInfo.PositionMessage;timestamp_utc=[DateTime]::UtcNow.ToString('o')}
    Write-VVEvent 'entry_failed' $detail
    $path=Join-Path $env:VV_REPORT_DIR 'failures.jsonl'
    [IO.File]::AppendAllText($path,($detail | ConvertTo-Json -Depth 12 -Compress)+"`n",[Text.UTF8Encoding]::new($false))
}
function New-VVStep([string]$Name) {
    if ([string]::IsNullOrWhiteSpace($env:VV_REPORT_DIR)) { throw 'A report session is required.' }
    $safe=$Name -replace '[^a-zA-Z0-9._-]','-'
    $path=Join-Path $env:VV_REPORT_DIR ('steps\'+[DateTime]::UtcNow.ToString('HHmmssfff')+'-'+$safe+'-'+[Guid]::NewGuid().ToString('N').Substring(0,4))
    [IO.Directory]::CreateDirectory($path) | Out-Null
    return $path
}
function Export-VVBundle([string]$RunRoot) {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $dest=Join-Path $RunRoot 'review.zip'; $tmp=$dest+'.new'
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force }
    $zip=[IO.Compression.ZipFile]::Open($tmp,[IO.Compression.ZipArchiveMode]::Create)
    try {
        $prefix=$RunRoot.TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
        foreach($file in @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File | Sort-Object FullName)) {
            if ($file.Name -in @('review.zip','review.zip.new','.active.lock') -or $file.Extension -eq '.new') { continue }
            if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            $rel=$file.FullName.Substring($prefix.Length).Replace('\','/')
            $compression=if($file.Extension -in @('.png','.xlsx','.zip')){[IO.Compression.CompressionLevel]::NoCompression}else{[IO.Compression.CompressionLevel]::Fastest}
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$file.FullName,$rel,$compression) | Out-Null
        }
    } finally { $zip.Dispose() }
    if (Test-Path -LiteralPath $dest) {
        try { [IO.File]::Replace($tmp,$dest,$null) }
        catch {
            # WSL/UNC providers may not implement ReplaceFile; all source artifacts remain intact.
            if (-not [IO.File]::Exists($tmp)) { throw }
            [IO.File]::Copy($tmp,$dest,$true)
            [IO.File]::Delete($tmp)
        }
    } else { [IO.File]::Move($tmp,$dest) }
}
# Read final workbook payloads first; only use progress when no final payload exists.
# Recovered tile errors are counted separately from terminal sheet failures.
function Get-VVCaptureRollup([string]$RunRoot) {
    $sessions=New-Object 'System.Collections.Generic.List[object]'
    $finalKeys=New-Object 'System.Collections.Generic.HashSet[string]'
    foreach($file in @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File -Filter '*.json' | Where-Object {$_.FullName -match '[\\/]data[\\/]'})) {
        try {
            $data=Read-VVJson $file.FullName
            foreach($property in @($data.workbooks.PSObject.Properties)) {
                $safe=$property.Value.safe_capture
                if($null -eq $safe){continue}
                [void]$finalKeys.Add(([string]$safe.workbook+'|'+[string]$safe.strategy))
                $sessions.Add([pscustomobject]@{Data=$safe;Source=$file.FullName})
            }
        } catch { Write-Warning ('Capture summary cannot read '+$file.FullName+': '+$_.Exception.Message) }
    }
    foreach($file in @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File -Filter '*.progress.json')) {
        try {
            $safe=Read-VVJson $file.FullName
            if($null -eq $safe.sheets -or $null -eq $safe.attempts){continue}
            $key=[string]$safe.workbook+'|'+[string]$safe.strategy
            if($finalKeys.Contains($key)){continue}
            [void]$finalKeys.Add($key)
            $sessions.Add([pscustomobject]@{Data=$safe;Source=$file.FullName})
        } catch { Write-Warning ('Capture progress cannot be read: '+$file.FullName) }
    }
    $complete=0; $failed=0; $inconclusive=0; $pending=0; $accepted=0; $attempts=0; $calibrationImages=0
    $sheetErrors=@();$attemptErrors=@();$calibrations=@()
    foreach($session in $sessions) {
        $safe=$session.Data
        $attempts+=@($safe.attempts | Where-Object {$null -ne $_}).Count
        foreach($attempt in @($safe.attempts | Where-Object {$null -ne $_.error})) {
            $attemptErrors+=[pscustomobject]@{code=[string]$attempt.error.code;sheet=[string]$attempt.sheet;workbook=[string]$safe.workbook}
        }
        foreach($cal in @($safe.calibrations | Where-Object {$null -ne $_})) {
            $calibrationImages+=@($cal.observations | Where-Object {$null -ne $_.png}).Count
            $calibrations+=[pscustomobject]@{workbook=$safe.workbook;sheet=$cal.sheet;status=$cal.status;scale_x=$cal.x.scale;scale_y=$cal.y.scale}
        }
        foreach($sheet in @($safe.sheets | Where-Object {$null -ne $_})) {
            switch([string]$sheet.status) {
                'complete' {$complete++}
                'failed' {$failed++}
                'inconclusive' {$inconclusive++}
                default {$pending++}
            }
            $accepted+=@($sheet.tiles | Where-Object {$null -ne $_ -and -not $_.excluded_probe_margin}).Count
            if($null -ne $sheet.error) {
                $sheetErrors+=[pscustomobject]@{workbook=[string]$safe.workbook;sheet=[string]$sheet.sheet;
                    code=[string]$sheet.error.code;message=[string]$sheet.error.message;detail=$sheet.error.detail}
            }
        }
    }
    $groups=@($attemptErrors | Group-Object code | ForEach-Object {
        [pscustomobject]@{code=$_.Name;attempt_count=$_.Count;sheets=@($_.Group.sheet | Sort-Object -Unique)}
    })
    return [pscustomobject]@{sessions=$sessions.Count;complete_sheets=$complete;failed_sheets=$failed;
        inconclusive_sheets=$inconclusive;pending_sheets=$pending;accepted_tiles=$accepted;
        rendered_tile_attempts=$attempts;calibration_images=$calibrationImages;
        sheet_errors=$sheetErrors;attempt_error_groups=$groups;calibrations=$calibrations}
}
function Complete-VVReport([string]$RunRoot, [switch]$Interrupted) {
    $meta=Read-VVJson (Join-Path $RunRoot 'run.json')
    if ($null -eq $meta -or $meta.schema -ne 'vv-report-v1') { throw 'Not a managed report.' }
    $prefix=$RunRoot.TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
    $errors=@(); $outputs=@(); $sheetRows=[Text.StringBuilder]::new()
    $failure=Join-Path $RunRoot 'failures.jsonl'
    if(Test-Path -LiteralPath $failure) { foreach($line in [IO.File]::ReadLines($failure)) { try{$errors+=($line|ConvertFrom-Json)}catch{} } }
    $partial=$false; $hasResult=$false
    foreach($file in @(Get-ChildItem -LiteralPath $RunRoot -Recurse -Filter '*.json' -File)) {
        if($file.DirectoryName -eq $RunRoot -or $file.FullName -match '[\\/]inputs[\\/]' -or $file.Name -match 'manifest|config|plan|files') {continue}
        try {
            $data=Read-VVJson $file.FullName
            if ($null -ne $data.captures -and @($data.captures | Where-Object {$null -ne $_.error -and [string]$_.error -ne ''}).Count -gt 0) {$partial=$true}
            $isOutput=$null -ne $data.workbooks -or $null -ne $data.captures -or $file.Name -like '*.progress.json' -or $file.Name -eq 'result.json'
            if(-not $isOutput){continue}
            $rel=$file.FullName.Substring($prefix.Length).Replace('\','/')
            $outputs+=[ordered]@{path=$rel;bytes=$file.Length}
            if($null -ne $data.workbooks) {
                if(@($data.workbooks.PSObject.Properties).Count -gt 0){$hasResult=$true}
                if($data._meta.completed_workbooks -lt $data._meta.expected_workbooks){$partial=$true}
                foreach($p in $data.workbooks.PSObject.Properties) {
                    $w=$p.Value
                    if($w.status -in @('failed','partial','inconclusive')){$partial=$true}
                    $label=[Net.WebUtility]::HtmlEncode([string]$p.Name)
                    $st=[Net.WebUtility]::HtmlEncode([string]$w.status)
                    [void]$sheetRows.AppendLine("<tr><td>$label</td><td>$st</td><td><a href='$rel'>Result</a></td></tr>")
                }
            }
            if($data.status -in @('failed','partial','inconclusive','skipped')){$partial=$true}
            if($null -ne $data.errors -and @($data.errors).Count -gt 0){$partial=$true}
        } catch { $partial=$true; $errors+=@{message='Unreadable partial JSON: '+$file.FullName;exception=$_.Exception.Message} }
    }
    # Native benchmark JSON uses another schema. Retain it without pretending it contains sheet screenshots.
    $native=@(Get-ChildItem -LiteralPath $RunRoot -Recurse -Filter '*.json' -File | Where-Object {
        $_.FullName -match '[\\/]data[\\/]' -and $_.Name -notmatch 'manifest|config|plan|files'
    })
    if($native.Count -gt 0){$hasResult=$true}
    $timeline=New-Object 'System.Collections.Generic.List[object]'
    foreach($journal in @(Get-ChildItem -LiteralPath $RunRoot -Recurse -File | Where-Object {$_.Name -eq 'events.jsonl' -or $_.Name -like '*.events.jsonl'})) {
        foreach($line in [IO.File]::ReadLines($journal.FullName)) {
            try {
                $event=$line | ConvertFrom-Json
                $time=if($null -ne $event.timestamp_unix_ms){[DateTimeOffset]::FromUnixTimeMilliseconds([long]$event.timestamp_unix_ms).UtcDateTime.ToString('o')}else{[string]$event.timestamp_utc}
                $timeline.Add([pscustomobject]@{timestamp_utc=$time;source_file=$journal.FullName.Substring($prefix.Length).Replace('\','/');stage=$event.stage;detail=$event.detail})
            } catch { $timeline.Add([pscustomobject]@{timestamp_utc='';source_file=$journal.Name;stage='truncated_journal_line';detail=$line.Substring(0,[Math]::Min(500,$line.Length))}) }
        }
    }
    $timelinePath=Join-Path $RunRoot 'timeline.jsonl'
    $stream=[IO.StreamWriter]::new($timelinePath,$false,[Text.UTF8Encoding]::new($false))
    try {foreach($event in @($timeline | Sort-Object timestamp_utc)){$stream.WriteLine(($event | ConvertTo-Json -Depth 30 -Compress))}}finally{$stream.Dispose()}
    $captureSummary=Get-VVCaptureRollup $RunRoot
    $terminalErrors=@($captureSummary.sheet_errors)
    $allCaptureFailed=$captureSummary.failed_sheets -gt 0 -and $captureSummary.complete_sheets -eq 0 -and $captureSummary.inconclusive_sheets -eq 0
    $status=if($Interrupted){'interrupted'}elseif($errors.Count -gt 0 -or $allCaptureFailed){'failed'}elseif($partial -or $terminalErrors.Count -gt 0){'partial'}elseif($hasResult){'complete'}else{'no-results'}
    foreach($e in $terminalErrors){$errors+=$e}
    $meta.status=$status
    $meta | Add-Member -NotePropertyName completed_utc -NotePropertyValue ([DateTime]::UtcNow.ToString('o')) -Force
    Write-VVJson (Join-Path $RunRoot 'run.json') $meta
    $result=[ordered]@{schema='vv-run-result-v1';status=$status;run=$meta;outputs=$outputs;errors=$errors;capture_summary=$captureSummary;
        note='The original result/progress JSONs are referenced, not flattened or discarded. PNGs may contain probe margins; use valid_rect_px in tiled manifests.'}
    Write-VVJson (Join-Path $RunRoot 'result.json') $result
    $files=@(Get-ChildItem -LiteralPath $RunRoot -Recurse -File | Where-Object {$_.Name -notin @('review.zip','review.zip.new','.active.lock')})
    $images=@($files|Where-Object{$_.Extension -eq '.png'})
    $sum="STATUS: $status`nEXPERIMENT: $($meta.experiment)`nRUN: $($meta.id)`nPNG FILES: $($images.Count)`nRESULT FILES: $($outputs.Count)`n`nSend review.zip from this directory. It includes existing partial results after failures.`nScreenshots may contain workbook data; logs contain local paths.`n"
    if($captureSummary.sessions -gt 0) {
        $sum+="`nCAPTURE SUMMARY`nACCEPTED TILES: $($captureSummary.accepted_tiles)`nCOMPLETE SHEETS: $($captureSummary.complete_sheets)`nFAILED SHEETS: $($captureSummary.failed_sheets)`nINCONCLUSIVE SHEETS: $($captureSummary.inconclusive_sheets)`nPENDING SHEETS: $($captureSummary.pending_sheets)`nRAW TILE ATTEMPTS: $($captureSummary.rendered_tile_attempts)`nCALIBRATION IMAGES: $($captureSummary.calibration_images)`n"
        foreach($group in $captureSummary.attempt_error_groups){$sum+="`nATTEMPT ERROR: $($group.code) x$($group.attempt_count); sheets: $($group.sheets -join ', ')`n"}
        foreach($cal in $captureSummary.calibrations){$sum+="RASTER SCALE: sheet=$($cal.sheet); status=$($cal.status); x=$($cal.scale_x); y=$($cal.scale_y) px/point`n"}
    }
    foreach($e in $errors){$sum+="`nERROR: $($e.code) sheet=$($e.sheet): $($e.message)`n"}
    [IO.File]::WriteAllText((Join-Path $RunRoot 'summary.txt'),$sum,[Text.UTF8Encoding]::new($false))
    $links=[Text.StringBuilder]::new()
    foreach($f in @($files | Where-Object {$_.Extension -in @('.json','.jsonl','.csv','.log','.txt')})) {
        $rel=$f.FullName.Substring($prefix.Length).Replace('\','/')
        [void]$links.AppendLine("<li><a href='$([Net.WebUtility]::HtmlEncode($rel))'>$([Net.WebUtility]::HtmlEncode($rel))</a></li>")
    }
    $img=[Text.StringBuilder]::new()
    foreach($f in $images) {
        $rel=$f.FullName.Substring($prefix.Length).Replace('\','/')
        [void]$img.AppendLine("<li><a href='$([Net.WebUtility]::HtmlEncode($rel))'>$([Net.WebUtility]::HtmlEncode($rel))</a></li>")
    }
    $html=@"
<!doctype html><html lang="en"><meta charset="utf-8"><title>Visual validation - $status</title>
<style>body{font:16px system-ui;max-width:1100px;margin:32px auto;padding:0 20px}pre{white-space:pre-wrap;background:#f3f5f7;padding:20px}li{margin:6px 0}td{padding:8px;border-bottom:1px solid #ccc}a{color:#145e9b}details{margin:20px 0}</style>
<h1>$([Net.WebUtility]::HtmlEncode([string]$meta.experiment)) - $status</h1><p><a href='review.zip'>Review bundle</a> | <a href='../../index.html'>All runs</a></p>
<pre>$([Net.WebUtility]::HtmlEncode($sum))</pre><table>$sheetRows</table>
<p>Interrupted calls and saved progress are recorded in timeline.jsonl and the progress snapshots. A started call without a completion record is not a successful capture.</p>
<p>Original tiles can include probe margins. Logical image extents are defined by valid_rect_px and sheet_offset_px in the capture manifest.</p>
<details open><summary>Results and logs</summary><ul>$links</ul></details><details><summary>Original images ($($images.Count))</summary><ul>$img</ul></details></html>
"@
    [IO.File]::WriteAllText((Join-Path $RunRoot 'report.html'),$html,[Text.UTF8Encoding]::new($false))
    try { Write-VVInspector $RunRoot } catch {
        [IO.File]::WriteAllText((Join-Path $RunRoot 'report-render-error.txt'),$_.Exception.ToString())
        Write-Warning ('The fallback report is available; inspector rendering failed: '+$_.Exception.Message)
    }
}
function Stop-VVReport($Owner) {
    if (-not $Owner.Owned) {return}
    try {
        if($Owner.Transcript){try{Stop-Transcript|Out-Null}catch{}}
        Complete-VVReport $Owner.Path
        try { Export-VVBundle $Owner.Path } catch {
            [IO.File]::WriteAllText((Join-Path $Owner.Path 'bundle-error.txt'),$_.Exception.ToString())
            Write-Warning ('ZIP creation failed; send the report directory instead: '+$Owner.Path)
        }
        Update-VVIndex $Owner.Reports
    } catch { Write-Warning ('Report finalization failed; all existing files remain at '+$Owner.Path+': '+$_.Exception.Message) }
    finally {
        $Owner.Lock.Dispose()
        $Owner.ExecutionLock.Dispose()
        if($null -eq $Owner.Previous){Remove-Item Env:VV_REPORT_DIR -ErrorAction SilentlyContinue}else{$env:VV_REPORT_DIR=$Owner.Previous}
        Write-Host ''
        Write-Host ('REPORT DIRECTORY: '+$Owner.Path)
        Write-Host ('REPORT:       '+(Join-Path $Owner.Path 'report.html'))
        if (Test-Path -LiteralPath (Join-Path $Owner.Path 'review.zip')) { Write-Host ('REVIEW BUNDLE:    '+(Join-Path $Owner.Path 'review.zip')) }
        else {Write-Host ('REVIEW BUNDLE:    Bundle unavailable; archive this directory: '+$Owner.Path)}
    }
}
