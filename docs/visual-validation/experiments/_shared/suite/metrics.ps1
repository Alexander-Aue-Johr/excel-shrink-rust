function Get-VVRelative([string]$Root,[string]$Path) {
    Initialize-VVReportIO
    $prefix=[VVReportIO]::FullPath($Root).TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
    $full=[VVReportIO]::FullPath($Path)
    if(-not $full.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Evidence path leaves its report directory.'}
    return $full.Substring($prefix.Length).Replace('\','/')
}
function Get-VVCaseMetrics([string]$CaseRoot,$Job,[switch]$VerifyPixels) {
    $workbooks=@();$documents=@();$errors=@();$captureRows=@();$pixelCache=@{};$pixelMs=0
    foreach($file in @(Get-VVChildItem -LiteralPath (Join-Path $CaseRoot 'data') -File -Recurse -Filter '*.json' -ErrorAction Stop)) {
        if($file.Name -match 'options|manifest|config|plan|files'){continue}
        try {$doc=Read-VVJson $file.FullName}catch{$errors+="Unreadable result: $($file.Name): $($_.Exception.Message)";continue}
        if($null -eq $doc){continue}
        foreach($e in @($doc.errors|Where-Object{$null -ne $_})){$errors+=('Result error in '+$file.Name+': '+($e|ConvertTo-Json -Depth 12 -Compress))}
        $documents += [pscustomobject]@{path=(Get-VVRelative $CaseRoot $file.FullName);status=$doc.status;meta=$doc._meta}
        if($null -eq $doc.workbooks){continue}
        foreach($entry in @($doc.workbooks.PSObject.Properties)) {
            $wb=$entry.Value;$safe=$wb.safe_capture
            if($null -eq $safe){
                $workbooks += [pscustomobject]@{workbook=$entry.Name;status=$wb.status;kind=$Job.candidate.kind;source=(Get-VVRelative $CaseRoot $file.FullName);
                    capture_count=$wb.capture_count;probe_count=$wb.probe_count;analysis=$null}
                continue
            }
            $sheets=@($safe.sheets | Where-Object{$null -ne $_})
            $attempts=@($safe.attempts | Where-Object{$null -ne $_})
            $checks=@($safe.sparse_overflow_checks | Where-Object{$null -ne $_})
            $blockers=0
            foreach($check in $checks){if($null -ne $check.write_confirmed_cells){$blockers += [int]$check.write_confirmed_cells}elseif($check.restoration -eq 'verified'){$blockers += [int]$check.blocker_count}}
            $workbooks += [pscustomobject]@{workbook=$entry.Name;status=$wb.status;kind='capture';source=(Get-VVRelative $CaseRoot $file.FullName);
                complete_sheets=@($sheets|Where-Object{$_.status -eq 'complete'}).Count;sheet_count=$sheets.Count;
                failed_sheets=@($sheets|Where-Object{$_.status -eq 'failed'}).Count;inconclusive_sheets=@($sheets|Where-Object{$_.status -eq 'inconclusive'}).Count;
                capture_count=$wb.capture_count;actual_image_request_batch_size=$wb.actual_image_request_batch_size;
                render_attempts=$attempts.Count;timings=$wb.safe_capture_timings;compute=$safe.compute_profile;environment=$safe.environment;
                capture_view=$wb.capture_view;capture_options=$safe.capture_options;capture_policy=$wb.capture_policy_version;
                verification_groups=$checks.Count;confirmed_blockers=$blockers;
                sheets=@($sheets|ForEach-Object{[pscustomobject]@{sheet=$_.sheet;sheet_index=$_.sheet_index;address=$_.address;status=$_.status;warning=$_.warning}})}
            $artifactRelative=(Get-VVRelative $CaseRoot $file.FullName) -replace '^data/','artifacts/' -replace '\.json$',''
            $artifactRoot=Join-Path $CaseRoot ($artifactRelative.Replace('/',[IO.Path]::DirectorySeparatorChar))
            $raw=@{};foreach($r in @($safe.captures)){if($null -ne $r.id){$raw[[string]$r.id]=$r}}
            foreach($tile in @($wb.captures)) {
                if($null -eq $tile -or $tile.excluded_probe_margin){continue}
                $sourceId=if($tile.source_id){[string]$tile.source_id}else{[string]$tile.id}
                $source=$raw[$sourceId]
                $p=[string]$tile.relative_png;$digest=$null;$issue=$null;$evidence=$null
                if($p -and $p -notmatch '(^[\\/]|:|(^|[\\/])\.\.([\\/]|$))') {
                    try {
                        $full=Join-Path $artifactRoot ($p.Replace('/',[IO.Path]::DirectorySeparatorChar))
                        $evidence=Get-VVRelative $CaseRoot $full
                        if($VerifyPixels) {
                            $sw=[Diagnostics.Stopwatch]::StartNew();$pixelStream=$null
                            try {
                                if(-not [VVReportIO]::FileExists($full)){throw ('Final PNG is absent: '+$evidence)}
                                $pixelStream=[VVReportIO]::OpenRead($full)
                                $hash=[VVReportIO]::HashStream($pixelStream)
                                if($tile.sha256 -and $hash -ne [string]$tile.sha256){throw 'Stored final-PNG SHA-256 does not match the local file.'}
                                if($pixelCache.ContainsKey($hash)){$digest=$pixelCache[$hash]}
                                else {
                                    $pixelStream.Position=0
                                    $d=[VVSuitePixels]::ReadStream($pixelStream,64000000)
                                    $digest=[pscustomobject]@{algorithm=$d.Algorithm;sha256=$d.Sha256;width=$d.Width;height=$d.Height}
                                    $pixelCache[$hash]=$digest
                                }
                                if($digest.width -ne $tile.png_width -or $digest.height -ne $tile.png_height){throw 'Final-PNG dimensions disagree with result metadata.'}
                            }finally{
                                if($null -ne $pixelStream){$pixelStream.Dispose()}
                                $sw.Stop();$pixelMs+=$sw.Elapsed.TotalMilliseconds
                            }
                        }
                    }catch{$digest=$null;$issue=$_.Exception.Message}
                }else{$issue='Missing or unsafe final-PNG path'}
                $captureRows += [pscustomobject]@{workbook=$entry.Name;sheet=$tile.sheet;sheet_index=$tile.sheet_index;
                    address=$tile.address;capture_address=$tile.capture_address;rect=$tile.rect;offset=$tile.sheet_offset_px;
                    width=$tile.png_width;height=$tile.png_height;raster=$tile.raster_profile;
                    encoded_sha256=$tile.sha256;raw_sha256=$(if($tile.raw_sha256){$tile.raw_sha256}else{$source.sha256});
                    raw_address=$source.address;pixel_digest=$digest;pixel_issue=$issue;path=$evidence}
            }
        }
    }
    $captureWbs=@($workbooks|Where-Object{$_.kind -eq 'capture'})
    $status='complete'
    if($documents.Count -eq 0){$status='no-results'}
    elseif($errors.Count -gt 0){$status='failed'}
    elseif(@($documents|Where-Object{$_.status -eq 'skipped'}).Count -gt 0){$status='skipped'}
    elseif(@($workbooks|Where-Object{$_.status -in @('failed','partial','inconclusive','running')}).Count -gt 0){$status='partial'}
    if($Job.candidate.kind -eq 'capture' -and ($captureWbs.Count -eq 0 -or @($captureWbs|Where-Object{$_.sheet_count -eq 0 -or $_.complete_sheets -ne $_.sheet_count}).Count -gt 0)){$status='partial'}
    return [pscustomobject]@{schema='vv-suite-case-metrics-v1';status=$status;workbooks=$workbooks;captures=$captureRows;
        documents=$documents;errors=$errors;pixel_verification_requested=[bool]$VerifyPixels;pixel_verification_ms=$pixelMs;
        pixel_verification_status=$(if(-not $VerifyPixels){'not-requested'}elseif($captureRows.Count -eq 0){'unverified'}elseif(@($captureRows|Where-Object{$null -eq $_.pixel_digest -or $_.pixel_issue}).Count -gt 0){'unverified'}else{'complete'});
        pixel_verified_captures=@($captureRows|Where-Object{$null -ne $_.pixel_digest -and -not $_.pixel_issue}).Count;
        pixel_unverified_captures=$(if($VerifyPixels){@($captureRows|Where-Object{$null -eq $_.pixel_digest -or $_.pixel_issue}).Count}else{0});
        timing_note='Workload wall time is measured by the case worker before offline pixel hashing and report rendering. Named phase timers can overlap; do not add them.'}
}
