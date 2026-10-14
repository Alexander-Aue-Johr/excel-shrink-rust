$script:VVSuiteAssetRoot=$PSScriptRoot
function Connect-VVCaseReport([string]$CaseRoot) {
    $asset=Join-Path $CaseRoot 'report-assets\inspector.js'
    if(Test-Path -LiteralPath $asset) {
        $text=[IO.File]::ReadAllText($asset)
        $text=$text.Replace("link('Full evidence','review.zip','button'),",'')
        $text=$text.Replace("el('a','button','All runs')","el('a','button','Suite overview')").Replace("overviewLink.href='../../index.html'","overviewLink.href='../../report.html'")
        $text=$text.Replace('A/B uses separate working copies. Fixed A-then-B order does not control cache/order effects.','Part of a configured comparison suite. See its overview for input, round and order controls.')
        [IO.File]::WriteAllText($asset,$text,[Text.UTF8Encoding]::new($false))
    }
}
function Write-VVSuiteView([string]$RunRoot,$State) {
    $assets=Join-Path $RunRoot 'report-assets';[IO.Directory]::CreateDirectory($assets)|Out-Null
    foreach($name in @('suite.css','suite.js')){Copy-Item -LiteralPath (Join-Path $script:VVSuiteAssetRoot $name) -Destination (Join-Path $assets $name) -Force}
    Copy-Item -LiteralPath (Join-Path $script:VVSuiteAssetRoot '..\report\inspector.css') -Destination (Join-Path $assets 'inspector.css') -Force
    $json=($State|ConvertTo-Json -Depth 100 -Compress).Replace('<','\u003c').Replace('>','\u003e').Replace('&','\u0026')
    $html=@"
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Comparison suite · Excel Shrink</title><link rel="stylesheet" href="report-assets/inspector.css"><link rel="stylesheet" href="report-assets/suite.css"></head>
<body><div id="vv-suite"></div><noscript>Enable JavaScript for the comparison tables. The same measurements are stored in suite-state.json and case-results.csv.</noscript><script type="application/json" id="vv-suite-data">$json</script><script src="report-assets/suite.js"></script></body></html>
"@
    [IO.File]::WriteAllText((Join-Path $RunRoot 'report.html.new'),$html,[Text.UTF8Encoding]::new($false))
    [IO.File]::Copy((Join-Path $RunRoot 'report.html.new'),(Join-Path $RunRoot 'report.html'),$true)
    [IO.File]::Delete((Join-Path $RunRoot 'report.html.new'))
}
function Complete-VVSuiteReport([string]$RunRoot,[switch]$Interrupted) {
    $meta=Read-VVJson (Join-Path $RunRoot 'run.json');$state=Read-VVJson (Join-Path $RunRoot 'suite-state.json')
    if($null -eq $state){$state=[pscustomobject]@{schema='vv-comparison-result-v1';profile='unknown';status='failed';jobs=@();errors=@('Suite setup failed before its plan was written.');catalog=@();inputs=@();run_id=$meta.id}}
    $recovered=$false
    foreach($job in @($state.jobs)) {
        $caseRoot=Join-Path $RunRoot ('cases\'+$job.id)
        if($job.status -in @('running','queued') -and $Interrupted) {
            $job.status=if($job.status -eq 'running'){'interrupted'}else{'not-run'};$recovered=$true
        }
        if($job.status -eq 'interrupted' -and (Test-Path -LiteralPath (Join-Path $caseRoot 'run.json')) -and -not (Test-VVRunActive $caseRoot)) {
            try{Complete-VVReport $caseRoot -Interrupted;Connect-VVCaseReport $caseRoot;Export-VVMetadataBundle $caseRoot}catch{Write-Warning ('Could not recover case '+$job.id+': '+$_.Exception.Message)}
        }
        $metric=Read-VVJson (Join-Path $caseRoot 'case-metrics.json')
        if($null -ne $metric){$job.metrics=$metric}
    }
    $pending=@($state.jobs|Where-Object{$_.status -in @('queued','running','not-run','interrupted')}).Count
    $bad=@($state.jobs|Where-Object{$_.status -notin @('complete','queued','running')}).Count
    $good=@($state.jobs|Where-Object{$_.status -eq 'complete'}).Count
    $status=if($Interrupted -or $pending -gt 0){'interrupted'}elseif(@($state.errors|Where-Object{$_}).Count -gt 0){'failed'}elseif($bad -gt 0){'partial'}elseif($good -gt 0){'complete'}else{'no-results'}
    $state.status=$status
    if(-not $state.completed_utc){$state|Add-Member -NotePropertyName completed_utc -NotePropertyValue ([DateTime]::UtcNow.ToString('o')) -Force}
    $meta.status=$status;$meta|Add-Member -NotePropertyName completed_utc -NotePropertyValue $state.completed_utc -Force
    Write-VVJson (Join-Path $RunRoot 'run.json') $meta
    Write-VVJson (Join-Path $RunRoot 'suite-state.json') $state
    $rollup=[ordered]@{complete_sheets=0;failed_sheets=0;inconclusive_sheets=0;pending_sheets=0;accepted_tiles=0}
    $csv=@()
    foreach($job in @($state.jobs)) {
        $render=0.0;$persist=0.0;$finalize=0.0;$telemetry=0.0;$tileCount=0;$sheets=0;$groups=0;$blockers=0
        foreach($wb in @($job.metrics.workbooks|Where-Object{$null -ne $_ -and $_.kind -eq 'capture'})) {
            $rollup.complete_sheets += [int]$wb.complete_sheets;$rollup.failed_sheets += [int]$wb.failed_sheets;$rollup.inconclusive_sheets += [int]$wb.inconclusive_sheets;$rollup.accepted_tiles += [int]$wb.capture_count
            $render += [double]$wb.timings.render_ms;$persist += [double]$wb.timings.persist_ms;$finalize += [double]$wb.timings.finalize_ms;$telemetry += [double]$wb.timings.telemetry_ms
            $tileCount += [int]$wb.capture_count;$sheets += [int]$wb.complete_sheets;$groups += [int]$wb.verification_groups;$blockers += [int]$wb.confirmed_blockers
        }
        $hasCapture=@($job.metrics.workbooks|Where-Object{$null -ne $_ -and $_.kind -eq 'capture'}).Count -gt 0
        $csv += [pscustomobject]@{sequence=$job.sequence;candidate=$job.candidate_id;phase=$job.phase;round=$job.round;status=$job.status;
            workload_ms=$job.metrics.workload_ms;offline_pixel_verification_ms=$job.metrics.pixel_verification_ms;
            render_ms=$(if($hasCapture){$render}else{$null});persist_ms=$(if($hasCapture){$persist}else{$null});finalize_ms=$(if($hasCapture){$finalize}else{$null});telemetry_ms=$(if($hasCapture){$telemetry}else{$null});
            complete_sheets=$sheets;tiles=$tileCount;verification_groups=$groups;blocker_cells=$blockers;report=$job.report;error=$job.error}
    }
    foreach($row in $csv){
        foreach($column in @('workload_ms','offline_pixel_verification_ms','render_ms','persist_ms','finalize_ms','telemetry_ms')){
            if($null -ne $row.$column){$row.$column=([double]$row.$column).ToString('R',[Globalization.CultureInfo]::InvariantCulture)}
        }
    }
    if($csv.Count){$csv|Export-Csv -LiteralPath (Join-Path $RunRoot 'case-results.csv') -NoTypeInformation -Encoding UTF8}
    $runErrors=@($state.errors|Where-Object{$_})
    foreach($failedJob in @($state.jobs|Where-Object{$_.status -notin @('complete','queued','running')})){$runErrors += [pscustomobject]@{case=$failedJob.id;status=$failedJob.status;message=$(if($failedJob.error){$failedJob.error}else{'Inspect the case report for '+$failedJob.status});report=$failedJob.report}}
    Write-VVJson (Join-Path $RunRoot 'result.json') ([ordered]@{schema='vv-run-result-v1';status=$status;run=$meta;capture_summary=$rollup;
        suite=[ordered]@{profile=$state.profile;total=$state.jobs.Count;complete=$good;requires_review=$bad;pending=$pending};
        outputs=@(@{path='suite-state.json'},@{path='case-results.csv'});errors=$runErrors;
        note='A suite completion is an execution outcome, not proof that all capture methods preserve identical visual content. See comparison gates and per-case reports.'})
    $summary="STATUS: $status`nPROFILE: $($state.profile)`nCASES: $($state.jobs.Count)`nCOMPLETE: $good`nREQUIRES CAPTURE REVIEW: $bad`nPENDING / INTERRUPTED: $pending`n`nSend this suite's review-metadata.zip. No images are required for routine review.`nRaw images remain under cases/. Warm-up trials are not included in performance statistics.`n"
    $unverified=@($state.jobs|Where-Object{
        $_.phase -eq 'measured' -and $_.metrics.pixel_verification_requested -and
        @($_.metrics.captures|Where-Object{$null -eq $_.pixel_digest -or $_.pixel_issue}).Count -gt 0
    }).Count
    $reportFailures=@($state.jobs|Where-Object{$_.metrics.report_publication.status -eq 'failed'}).Count
    $summary+="`nPOST-PROCESSING`nMEASURED CASES WITH UNVERIFIED PIXELS: $unverified`nCASE REPORT FAILURES: $reportFailures`nCapture status and offline verification are separate outcomes. Bundle publication is recorded in bundle-status.json.`n"
    [IO.File]::WriteAllText((Join-Path $RunRoot 'summary.txt'),$summary,[Text.UTF8Encoding]::new($false))
    Write-VVSuiteView $RunRoot $state
}
