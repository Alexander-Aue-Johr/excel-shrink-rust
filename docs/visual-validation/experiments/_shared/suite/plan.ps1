# Explicit experiment catalog. No arbitrary command strings are accepted from suite.json.
function Get-VVSuiteCatalog {
    $out=@()
    foreach($mode in @('heuristic','selective')) {
        foreach($hostKind in @('dotnet','rust')) {
            $out += [pscustomobject]@{ id="f-$hostKind-$mode";kind='capture';strategy='F';host_kind=$hostKind;overflow_mode=$mode;
                group='current-capture';label="Strategy F / $hostKind / $mode";scope='Current candidate';
                purpose='Compare native hosts and overflow verification with identical source workbooks and capture settings.' }
        }
    }
    foreach($strategy in @('A','B','C','D','E')) {
        $out += [pscustomobject]@{id=($strategy.ToLowerInvariant()+'-dotnet');kind='capture';strategy=$strategy;host_kind='dotnet';overflow_mode='heuristic';
            group='bounds-baselines';label="Strategy $strategy / dotnet";scope='Historical baseline';
            purpose='Screen alternative bounds planning once. Different extents are not automatically equivalent visual coverage.'}
    }
    $out += [pscustomobject]@{id='vba-picture';kind='vba-picture';group='native-picture';label='VBA CopyPicture / Strategy A';scope='Native reference';
        purpose='Native screenshot baseline. Not automatically comparable with normalized Office.js gridlines, raster scale or tiled bounds.'}
    $out += [pscustomobject]@{id='shape-native-base';kind='shape-native';group='drawing-bounds';label='Shapes / VBA + Excel-DNA + PowerShell COM + Rust COM';scope='Native reference group';
        purpose='The existing native runner executes these four implementations together. Invoke it once, not four times.'}
    $out += [pscustomobject]@{id='shape-vsto';kind='shape-vsto';group='drawing-bounds';label='Shapes / VSTO';scope='Optional installed reference';
        purpose='Run the existing VSTO benchmark when its prerequisites are available. Skips remain visible.'}
    foreach($hostKind in @('dotnet','rust')) {
        $out += [pscustomobject]@{id="shape-officejs-$hostKind";kind='shape-officejs';host_kind=$hostKind;group='drawing-bounds';
            label="Shapes / Office.js / $hostKind";scope='Current drawing candidate';purpose='Measure shape geometry, not screenshot rendering.'}
    }
    foreach($suite in @('units','markers','row-heights','height-plateau')) {
        $out += [pscustomobject]@{id=$suite;kind='diagnostic';suite=$suite;group='renderer-diagnostics';label="Diagnostic / $suite";
            scope='On-demand diagnostic';purpose='Explain renderer behavior. A completed diagnostic may still have an inconclusive scientific conclusion.'}
    }
    return $out
}
function Get-VVSuitePlan($Config,[string]$Profile) {
    if($Config.schema -ne 'vv-comparison-suite-v1'){throw 'Unsupported comparison-suite schema.'}
    if([string]::IsNullOrWhiteSpace($Profile)){$Profile=[string]$Config.default_profile}
    if($Profile -notmatch '^[a-z][a-z0-9-]{0,40}$'){throw 'Invalid suite profile.'}
    $property=$Config.profiles.PSObject.Properties[$Profile]
    if($null -eq $property){throw "Unknown suite profile: $Profile"}
    $profileConfig=$property.Value
    $warm=[int]$profileConfig.warmups;$reps=[int]$profileConfig.repetitions
    if($warm -lt 0 -or $warm -gt 2 -or $reps -lt 1 -or $reps -gt 10){throw 'Use 0..2 warm-ups and 1..10 measured rounds.'}
    if([int]$Config.capture.batch_size -ne 1){throw 'The suite fixes logical batch size at 1; native safe-capture calls are currently sequential.'}
    if($Config.capture.excel_launch_mode -ne 'excel-x'){throw 'The suite fixes Excel launch mode at excel-x for comparable host runs.'}
    if([int]$Config.capture.tile_megapixels -notin @(16,32,64)){throw 'Tile budget must be 16, 32 or 64 MP.'}
    if($Config.capture.tile_packing -notin @('budget','balanced')){throw 'Invalid tile packing.'}
    foreach($v in @($Config.capture.overflow_probe_width_px,$Config.capture.overflow_probe_height_px)) {
        if([int]$v -lt 64 -or [int]$v -gt 8192){throw 'Probe dimensions must be 64..8192 pixels.'}
    }
    if([int]$Config.execution.case_timeout_seconds -lt 60 -or [int]$Config.execution.case_timeout_seconds -gt 7200){throw 'Case timeout must be 60..7200 seconds.'}
    if([int]$Config.execution.cooldown_seconds -lt 0 -or [int]$Config.execution.cooldown_seconds -gt 60){throw 'Cooldown must be 0..60 seconds.'}
    if($Config.execution.stop_on_input_change -ne $true){throw 'Input changes must stop this comparison suite.'}
    if(@($Config.workbooks).Count -lt 1){throw 'Specify at least one workbook path, or ["*"] for the entire corpus.'}
    $catalog=@(Get-VVSuiteCatalog);$chosen=@();$seen=@{}
    foreach($id in @($profileConfig.cases)) {
        if($seen.ContainsKey([string]$id)){throw "Duplicate case in profile: $id"};$seen[[string]$id]=$true
        $c=@($catalog|Where-Object{$_.id -eq $id})
        if($c.Count -ne 1){throw "Unknown suite case: $id"};$chosen+=$c[0]
    }
    if($chosen.Count -eq 0){throw 'The profile is empty.'}
    $jobs=@();$n=0
    foreach($phase in @('warmup','measured')) {
        $rounds=if($phase -eq 'warmup'){$warm}else{$reps}
        for($round=1;$round -le $rounds;$round++) {
            # Cyclic order is deterministic and recorded. Three rounds do not completely balance four positions.
            $shift=if($phase -eq 'warmup'){0}else{($round-1)%$chosen.Count}
            for($position=0;$position -lt $chosen.Count;$position++) {
                $c=$chosen[($position+$shift)%$chosen.Count];$n++
                $jobs += [pscustomobject]@{sequence=$n;id=('{0:d3}-{1}-{2}{3}' -f $n,$c.id,$phase,$round);candidate_id=$c.id;
                    phase=$phase;round=$round;position=($position+1);candidate=$c;status='queued';report=$null;metrics=$null;error=$null}
            }
        }
    }
    if($jobs.Count -gt 250){throw 'Suite exceeds the 250-case safety limit.'}
    return [pscustomobject]@{schema='vv-suite-plan-v1';profile=$Profile;description=[string]$profileConfig.description;
        warmups=$warm;repetitions=$reps;catalog=$catalog;selected=$chosen;jobs=$jobs;
        capture=$Config.capture;ordering='cyclic rotation per measured round; no claim of randomization or complete order balance'}
}
function Get-VVSuiteInputs([string]$RepoRoot,$Config,[string]$CorpusOverride) {
    $corpus=if([string]::IsNullOrWhiteSpace($CorpusOverride)){[string]$Config.corpus_root}else{$CorpusOverride}
    if(-not [IO.Path]::IsPathRooted($corpus)){$corpus=Join-Path $RepoRoot $corpus}
    $corpus=[IO.Path]::GetFullPath($corpus)
    if(-not(Test-Path -LiteralPath $corpus -PathType Container)){throw "Corpus directory not found: $corpus"}
    $all=@(Get-ChildItem -LiteralPath $corpus -File -Recurse -Filter '*.xlsx' | Where-Object{ -not $_.Name.StartsWith('~$') -and -not($_.Attributes -band [IO.FileAttributes]::ReparsePoint)} | Sort-Object FullName)
    $selected=@();$names=@{}
    if(@($Config.workbooks).Count -eq 1 -and $Config.workbooks[0] -eq '*'){$selected=$all}
    else {
        foreach($name in @($Config.workbooks)) {
            if([string]::IsNullOrWhiteSpace($name) -or $name -match '[*?]'){throw 'Use exact paths, or a single "*" to select the whole corpus.'}
            $p=if([IO.Path]::IsPathRooted([string]$name)){[IO.Path]::GetFullPath([string]$name)}else{[IO.Path]::GetFullPath((Join-Path $corpus ([string]$name)))}
            $hits=@($all|Where-Object{[string]::Equals($_.FullName,$p,[StringComparison]::OrdinalIgnoreCase)})
            if($hits.Count -eq 0 -and [IO.Path]::GetFileName([string]$name) -eq [string]$name){$hits=@($all|Where-Object{$_.Name -eq $name})}
            if($hits.Count -ne 1){throw "Workbook selection is missing or ambiguous inside the corpus: $name"}
            $selected+=$hits[0]
        }
    }
    if($selected.Count -eq 0){throw 'No workbook selected.'}
    foreach($f in $selected){if($names.ContainsKey($f.Name)){throw "Duplicate workbook filename cannot be compared safely: $($f.Name)"};$names[$f.Name]=$true}
    return [pscustomobject]@{corpus=$corpus;files=$selected}
}
