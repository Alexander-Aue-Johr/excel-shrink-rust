# Rust is the already-running parent process. Only one request is outstanding.
$script:CombinedOverflowRequestId = 0L
function Invoke-CombinedOverflowAnalysis($capture, [int]$sheetIndex, [int]$baseColumn, [int]$endColumn, [int]$extra) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $script:CombinedOverflowRequestId++
    $id=$script:CombinedOverflowRequestId
    $background=[Drawing.Color]::FromArgb([int]$script:OverflowBackground)
    $capture | Add-Member -NotePropertyName sheet_index -NotePropertyValue $sheetIndex -Force
    $capture | Add-Member -NotePropertyName sheet_name -NotePropertyValue '' -Force
    $request=[pscustomobject]@{
        id=$id;sheet_index=$sheetIndex;base_column=$baseColumn;end_column=$endColumn;extra=$extra
        background_rgba=[pscustomobject]@{r=[int]$background.R;g=[int]$background.G;b=[int]$background.B;a=255}
        capture=$capture
    }
    $pending=Join-Path $PSScriptRoot 'overflow-request.pending'
    $published=Join-Path $PSScriptRoot 'overflow-request.json'
    $reply=Join-Path $PSScriptRoot "overflow-response-$id.json"
    [IO.File]::WriteAllText($pending, (ConvertTo-Json -InputObject $request -Depth 16 -Compress), [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($pending,$published)
    while(-not [IO.File]::Exists($reply)) {
        if($watch.Elapsed.TotalSeconds -ge 300){throw "Rust overflow analysis timed out request=$id"}
        Start-Sleep -Milliseconds 5
    }
    $response=[IO.File]::ReadAllText($reply) | ConvertFrom-Json
    [IO.File]::Delete($reply)
    if([long]$response.id -ne $id){throw 'Rust overflow response ID mismatch'}
    if($response.error){throw "Rust overflow analysis failed: $($response.error)"}
    Write-ProgressLine "visual step phase=excel.overflow.roundtrip duration_ms=$($watch.ElapsedMilliseconds) sheet=$sheetIndex request=$id columns=$endColumn"
    return $response
}

function Capture-WorksheetPair([int]$index, $rightSheet, $limit, $leftCache, $rightCache) {
    if($null -eq $limit -or $null -eq $limit.overflow_first_column){throw 'Rust column plan is required for combined overflow capture'}
    $baseColumn=[int]$limit.max_column
    $lastRow=[int]$limit.max_row
    $endColumn=[int]$limit.overflow_first_column
    if($endColumn -lt $baseColumn -or $endColumn -gt 16384){throw 'Invalid overflow column plan'}
    if($endColumn -eq $baseColumn) {
        Write-ProgressLine "overflow skipped sheet=$index base=$baseColumn reason='no visible columns to the right or XFD reached'"
        return Capture-WorksheetPairWithoutOverflow $index $rightSheet $limit $leftCache $rightCache
    }
    Ensure-OriginalWorkbook
    $leftSheet=$original.Worksheets.Item($index)
    if($leftSheet.Name -ne $rightSheet.Name){throw "Worksheet order/name differs at index $index"}
    $extra=1;$attempt=0
    while($true) {
        $attempt++
        $leftRange=$null;$rightRange=$null
        try {
            $leftRange=Worksheet-RangeByBounds $leftSheet 1 1 $lastRow $endColumn
            $rightRange=Worksheet-RangeByBounds $rightSheet 1 1 $lastRow $endColumn
            $useLeft=if(Test-CaptureGeometry $leftCache $leftRange){$leftCache}else{$null}
            $useRight=if(Test-CaptureGeometry $rightCache $rightRange){$rightCache}else{$null}
            if($null -ne $useLeft -and $null -ne $useRight -and -not (Test-CompatibleCachedPair $useLeft $useRight)) {
                $useLeft=$null;$useRight=$null
            }
            Write-ProgressLine "overflow combined sheet=$index attempt=$attempt base=$baseColumn end=$endColumn visible_extra_budget=$extra"
            # Unique attempt names retain previous images until the decision is complete.
            $baseName='{0:D3}-probe-{1:D2}' -f $index,$attempt
            $set=Capture-RangeImageSetWithPartialCache $leftSheet $rightSheet $leftRange $rightRange $OriginalCacheDir $ShrunkCacheDir $baseName "sheet $index combined attempt=$attempt" 'combined_overflow_proportional' $null $useLeft $useRight
        } finally {
            Release-OverflowCom $leftRange
            Release-OverflowCom $rightRange
        }
        $response=Invoke-CombinedOverflowAnalysis $set $index $baseColumn $endColumn $extra
        if($response.done) {
            Write-ProgressLine "overflow combined finished sheet=$index attempts=$attempt captured_column=$endColumn keep_column=$($response.keep_column)"
            return $response.capture
        }
        $next=[int]$response.next_column
        if($next -le $endColumn -or $next -gt 16384){throw 'Rust overflow planner made no progress'}
        $endColumn=$next;$extra=[int]$response.next_extra
    }
}
