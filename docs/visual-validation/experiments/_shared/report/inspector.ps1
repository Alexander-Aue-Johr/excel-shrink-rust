# Report projection only. Never invokes Excel, mutates measurement data, or decodes screenshots.
$script:VVInspectorAssetRoot = $PSScriptRoot
function Select-VVInspectorFields($Value, [string[]]$Names) {
    $out=[ordered]@{}
    foreach($name in $Names) {
        if($null -eq $Value){continue}
        if($Value -is [System.Collections.IDictionary]) {
            if($Value.Contains($name)){$out[$name]=$Value[$name]}
        } elseif($null -ne $Value.PSObject.Properties[$name]) {$out[$name]=$Value.$name}
    }
    return $out
}
function ConvertTo-VVInspectorWorkbook($Value,[string]$Name) {
    $out=Select-VVInspectorFields $Value @('workbook','status','strategy','safe_capture_timings','environment','captures','probes','precomputed_wsl_rust_visual_plan')
    if(-not $out.Contains('workbook')){$out.workbook=$Name}
    $out.capture_plans=@($Value.capture_plans | Where-Object {$null -ne $_} | ForEach-Object {
        Select-VVInspectorFields $_ @('id','sheet','sheet_index','office_position','address','initial_address','base_right_column','bottom_row','end_column',
        'content_right_column','content_bottom_row','drawing_right_column','drawing_bottom_row','drawing_right_points','drawing_bottom_points',
        'visible_shape_count','right_bound_source','bottom_bound_source','trailing_hidden_column_start','trailing_hidden_row_start','overflow_probe','overflow_checked','overflow_passes')
    })
    if($null -ne $Value.safe_capture) {
        $out.safe_capture=Select-VVInspectorFields $Value.safe_capture @('schema','workbook','strategy','status','implementation','limits','sheets','attempts','captures','calibrations',
            'sheet_catalog','bounds_history','precomputed_wsl_rust_visual_plan','environment','current','summary','capture_options','tile_plans','sparse_overflow_checks','sparse_overflow_scans','capture_view','capture_policy_version','final_exports','compute_profile','geometry_diagnostics','sheet_continuations','capture_scope','checkpoint_policy')
        # The final flattened list repeats the same tiles already available in the capture session.
        $out.Remove('captures');$out.Remove('probes')
    }
    return $out
}
function Get-VVInspectorFiles([string]$RunRoot) {
    $stack=[System.Collections.Generic.Stack[string]]::new();$stack.Push([IO.Path]::GetFullPath($RunRoot))
    while($stack.Count -gt 0) {
        $dir=$stack.Pop()
        foreach($entry in @(Get-VVChildItem -LiteralPath $dir -Force)) {
            if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){continue}
            if($entry.PSIsContainer){if($entry.Name -ne 'report-assets'){$stack.Push($entry.FullName)}}
            elseif($entry.Name -notin @('review.zip','review.zip.new','review-metadata.zip','review-metadata.zip.new','.active.lock','report-data.json') -and $entry.Extension -ne '.new'){$entry}
        }
    }
}
function Write-VVInspector([string]$RunRoot) {
    $root=[IO.Path]::GetFullPath($RunRoot)
    $meta=Read-VVJson (Join-Path $root 'run.json')
    if($null -eq $meta -or $meta.schema -ne 'vv-report-v1'){throw 'Not a managed visual-validation report.'}
    $prefix=$root.TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
    $files=@(Get-VVInspectorFiles $root | Sort-Object FullName)
    $inventory=[System.Collections.Generic.List[object]]::new()
    $documents=[System.Collections.Generic.List[object]]::new()
    $plans=[System.Collections.Generic.List[object]]::new()
    $notes=[System.Collections.Generic.List[string]]::new()
    foreach($file in $files) {
        $rel=$file.FullName.Substring($prefix.Length).Replace('\','/')
        $record=[ordered]@{path=$rel;bytes=$file.Length}
        if($file.Extension -eq '.png') {
            $handle=$null
            try {
                $handle=[VVReportIO]::OpenRead($file.FullName)
                $header=New-Object byte[] 24
                if($handle.Read($header,0,24) -eq 24 -and $header[0] -eq 137 -and $header[1] -eq 80 -and $header[2] -eq 78 -and $header[3] -eq 71) {
                    $record.width=[long]$header[16]*16777216+[long]$header[17]*65536+[long]$header[18]*256+$header[19]
                    $record.height=[long]$header[20]*16777216+[long]$header[21]*65536+[long]$header[22]*256+$header[23]
                }
            } catch {$notes.Add('PNG header unavailable: '+$rel)} finally {if($null -ne $handle){$handle.Dispose()}}
        }
        $inventory.Add($record)
        if($file.Extension -ne '.json' -or $file.DirectoryName -eq $root){continue}
        if($rel -notmatch '^(data|steps|inputs/plans|artifacts)/'){continue}
        if($rel -match '^artifacts/' -and $file.Name -notlike '*.progress.json'){continue}
        try {
            $data=Read-VVJson $file.FullName
            if($rel -match '^inputs/plans/' -and $null -ne $data.workbooks) {
                $plans.Add([ordered]@{path=$rel;sha256=[VVReportIO]::HashFile($file.FullName);document=$data})
            } elseif($null -ne $data.workbooks -and $data.workbooks -isnot [array]) {
                $w=@($data.workbooks.PSObject.Properties | ForEach-Object {ConvertTo-VVInspectorWorkbook $_.Value $_.Name})
                $documents.Add([ordered]@{path=$rel;meta=$data._meta;workbooks=$w})
            } elseif($file.Name -like '*.progress.json') {
                if($data.schema -like 'safe-capture*'){$documents.Add([ordered]@{path=$rel;progress=$data})}
                elseif($null -ne $data.workbook){$documents.Add([ordered]@{path=$rel;progress=(ConvertTo-VVInspectorWorkbook $data $data.workbook)})}
            } elseif($null -ne $data.captures -and $null -ne $data.workbook) {
                $documents.Add([ordered]@{path=$rel;workbooks=@((ConvertTo-VVInspectorWorkbook $data $data.workbook))})
            }
        } catch {$notes.Add('Metadata unavailable: '+$rel+' - '+$_.Exception.Message)}
    }
    $events=[System.Collections.Generic.List[object]]::new()
    $timeline=Join-Path $root 'timeline.jsonl'
    if(Test-Path -LiteralPath $timeline -PathType Leaf) {
        foreach($line in [IO.File]::ReadLines($timeline)) {
            try {
                $event=$line|ConvertFrom-Json
                if($event.stage -in @('bounds_stage','worksheet_catalog','sheet_failed','tile_failed')){$events.Add($event)}
            } catch {$notes.Add('An incomplete timeline record was omitted from the HTML view. The original log is retained.')}
        }
    }
    $data=[ordered]@{schema='vv-inspector-v1';meta=$meta;result=(Read-VVJson (Join-Path $root 'result.json'));documents=@($documents.ToArray());
        plans=@($plans.ToArray());events=@($events.ToArray());files=@($inventory.ToArray());warnings=@($notes.ToArray())}
    $json=$data|ConvertTo-Json -Depth 100 -Compress
    # Prevent worksheet names or diagnostic text from terminating the inert JSON script element.
    $json=$json.Replace('&','\u0026').Replace('<','\u003c').Replace('>','\u003e').Replace([string][char]0x2028,'\u2028').Replace([string][char]0x2029,'\u2029')
    $assets=Join-Path $root 'report-assets';[IO.Directory]::CreateDirectory($assets)|Out-Null
    foreach($name in @('inspector.js','inspector.css')) {
        [IO.File]::Copy((Join-Path $script:VVInspectorAssetRoot $name),(Join-Path $assets $name),$true)
    }
    $html=@"
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Excel Shrink | Visual validation</title><link rel="stylesheet" href="report-assets/inspector.css">
<script src="report-assets/inspector.js" defer></script></head><body>
<p id="vv-loading">Loading capture report... <a href="result.json">Run data</a> | <a href="summary.txt">Summary</a></p>
<noscript>This report requires JavaScript for grouping and lazy image loading. <a href="result.json">Open run data</a>.</noscript>
<div id="vv-report"></div><script id="vv-report-data" type="application/json">$json</script></body></html>
"@
    $tmp=Join-Path $root 'report.html.new';$dest=Join-Path $root 'report.html'
    [IO.File]::WriteAllText($tmp,$html,[Text.UTF8Encoding]::new($false))
    if([IO.File]::Exists($dest)) {
        try{[IO.File]::Replace($tmp,$dest,$null)}catch{[IO.File]::Copy($tmp,$dest,$true);[IO.File]::Delete($tmp)}
    } else {[IO.File]::Move($tmp,$dest)}
}
