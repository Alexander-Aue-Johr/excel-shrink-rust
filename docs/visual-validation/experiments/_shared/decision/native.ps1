# Headless bridge to the existing VBA-local GDI+/row implementation.
# Each measurement is ONE Excel.Run batch, not a new per-cell/per-image COM loop.
function Invoke-VVDecisionNative($Spec) {
    & {
        param($s)
        $case=[string]$s.case_root;$tool=Join-Path $s.repo_root 'tools\excel-lab'
        . (Join-Path $tool 'ExcelInterop.ps1')
        . (Join-Path $tool 'Plan.ps1')
        Set-StrictMode -Off
        $kind=[string]$s.job.candidate.kind
        $token=[string]$s.release_token
        $summary=[ordered]@{schema='vv-native-decision-v1';status='failed';kind=$kind;image_persistence=($s.job.candidate.id -ne 'lab-memory');clipboard_used=($kind -ne 'lab-rows' -and $kind -ne 'lab-setup');capture_count=0;failure_count=0;row_count=0;memory_equal=$null;scope='Existing native reference, not assumed equivalent to Office.js';error=$null}
        $session=$null;$helper=$null;$local=$null
        try {
            if($kind -eq 'lab-setup') {
                try {$hostInfo=Get-LabHost $tool;$summary.status='complete';$summary.reused=$true}
                catch {
                    if(-not $s.build_lab_host_if_needed){throw 'VBA host missing/stale and automatic reviewed-source build is disabled.'}
                    $powershell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
                    $buildRoot=Join-Path $case 'lab-build';[IO.Directory]::CreateDirectory($buildRoot)|Out-Null
                    # The build worker is a descendant of the supervisor Job Object. COM ownership
                    # is also published by the shared Start-LabExcel registration hook.
                    & $powershell -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File (Join-Path $tool 'worker.ps1') `
                        -Action build -ToolRoot $tool -RunRoot $buildRoot -StateFile (Join-Path $case 'lab-owner.json') -OwnerToken $token
                    if($LASTEXITCODE -ne 0){throw 'Reviewed-source VBA host build failed. No security policy was changed.'}
                    $hostInfo=Get-LabHost $tool;$summary.status='complete';$summary.reused=$false
                }
                $summary.host_sha256=(Get-FileHash -LiteralPath $hostInfo.Path -Algorithm SHA256).Hash.ToLowerInvariant()
                return
            }
            $hostInfo=Get-LabHost $tool
            $local=Join-Path $env:LOCALAPPDATA ('ExcelShrinkDecision\'+$s.job.id+'-'+$token.Substring(0,8))
            [IO.Directory]::CreateDirectory($local)|Out-Null
            $out=Join-Path $local 'artifacts';[IO.Directory]::CreateDirectory($out)|Out-Null
            # Controller can recover partial images even if a native COM call never returns.
            Write-VVJson (Join-Path $case 'native-location.json') @{owner_token=$token;local_root=$local;artifacts=$out;note='Private workspace; input copies are never saved back.'}
            $source=Join-Path $s.staged_corpus $s.inputs[0].name
            $original=Join-Path $local 'original.xlsx';[IO.File]::Copy($source,$original,$false)
            if((Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash.ToLowerInvariant() -ne $s.inputs[0].sha256){throw 'Native working-copy hash mismatch'}
            $other=''
            if($kind -eq 'lab-memory'){$other=Join-Path $local 'unchanged-copy.xlsx';[IO.File]::Copy($original,$other,$false)}
            $entries=@()
            if($kind -in @('lab-capture','lab-memory')) {
                $reference=Read-VVJson (Join-Path $s.reference_case_root 'case-metrics.json')
                $sheets=@($reference.workbooks | Where-Object {$_.kind -eq 'capture'} | ForEach-Object {$_.sheets} | Where-Object {$_.status -eq 'complete'})
                if($sheets.Count -eq 0){throw 'No complete reference worksheet bounds; will not scan an inflated UsedRange instead.'}
                foreach($sheet in $sheets){
                    Test-LabAddress $sheet.address
                    $entries += [pscustomobject]@{pair='000000';sheet=$sheet.sheet;range=$sheet.address;appearance=1;
                        format=$(if($s.job.candidate.id -eq 'lab-emf'){-4147}else{2});storage='png';transparent=$false}
                }
            }
            $options=@{shapes=$false;hash_values=$false;compare_values=$false;row_scope=$(if($kind -eq 'lab-rows'){'used-union'}else{'none'});
                screenshots=($kind -ne 'lab-rows');storage='png';copy_matrix=$false;transparent_emf=$false;compare_visual_vba=($kind -eq 'lab-memory');
                compare_visual_rust=$false;block_cells=65536;max_cells=20000000;tile_points=[int]$s.native.tile_points;max_tiles=[int]$s.native.max_tiles;max_differences=1000}
            $pairs=@(@{id='000000';name=$s.inputs[0].name;original=$original;shrunk=$other})
            $external=[pscustomobject]@{captures=@($entries);overflow=@()}
            $plan=New-LabPlan -Configuration $options -Pairs $pairs -External $external
            $plan.DocumentElement.SetAttribute('output',$out)
            $plan.DocumentElement.SetAttribute('persist_images',$(if($s.job.candidate.id -eq 'lab-memory'){'false'}else{'true'}))
            $plan.DocumentElement.SetAttribute('hide_gridlines','true')
            $request=Join-Path $local 'request.xml';$plan.Save($request)
            Write-VVJson (Join-Path $case 'native-plan.json') @{bounds_source=$s.reference_case_root;entries=$entries;options=$options;unchanged_pair=($kind -eq 'lab-memory');input_sha256=$s.inputs[0].sha256}
            $session=Start-LabExcel -StateFile (Join-Path $case 'lab-owner.json') -OwnerToken $token
            $localHost=Join-Path $local 'LabHost.xlsm';[IO.File]::Copy($hostInfo.Path,$localHost,$false)
            $helper=Open-LabWorkbook -Excel $session.Excel -Books $session.Books -Path $localHost -Helper
            $prefix="'$($helper.Name.Replace("'","''"))'!"
            if([string]$session.Excel.Run(($prefix+'LabBuild.Signature')) -cne $hostInfo.Signature){throw 'Loaded helper signature differs'}
            $session.Excel.Visible=($kind -ne 'lab-rows')
            $answer=$session.Excel.Run(($prefix+'LabMain.RunBatch'),$request,(Join-Path $case 'native-progress.log'),(Join-Path $case '.cancel'))
            if([string]$answer -cne 'ok' -or -not [IO.File]::Exists((Join-Path $out 'measurement-complete.json'))){throw 'Native batch did not confirm completion'}
            $caps=Read-VVJson (Join-Path $out 'captures.json')
            $failures=@($caps.captures | Where-Object {$_.status -eq 'failed'})
            $summary.capture_count=@($caps.captures).Count;$summary.failure_count=$failures.Count
            if($kind -eq 'lab-rows') {
                $rows=Read-VVJson (Join-Path $out 'original.json')
                foreach($wb in @($rows)){foreach($sheet in @($wb.sheets)){$summary.row_count += [long]$sheet.row_heights.measured_rows}}
                if($summary.row_count -eq 0){throw 'Row-height batch produced no measured rows'}
            }
            if($kind -eq 'lab-memory') {
                $pairs=@($caps.captures | Where-Object {$null -ne $_.vba_visual_compare})
                $summary.memory_equal=($pairs.Count -gt 0 -and @($pairs | Where-Object {$_.vba_visual_compare.equal -ne $true}).Count -eq 0)
                if(-not $summary.memory_equal){throw 'Unchanged-workbook memory comparison differed or was not executed'}
            }
            $summary.status=if($failures.Count){'partial'}elseif($kind -eq 'lab-rows' -or $summary.capture_count -gt 0){'complete'}else{'failed'}
            $summary.timings=Read-VVJson (Join-Path $out 'timings.json')
        } catch {$summary.error=$_.Exception.Message;$summary.status=if($kind -eq 'lab-setup'){'unavailable'}else{'failed'};throw}
        finally {
            if($null -ne $helper){try{$helper.Close($false)}catch{};Release-LabCom $helper}
            if($null -ne $session){Close-LabExcel $session}
            Write-VVJson (Join-Path $case 'native-summary.json') $summary
            # Preserve/copy after measurement through the long-path-safe report I/O layer.
            if($local -and [IO.Directory]::Exists((Join-Path $local 'artifacts'))){
                $nativeRoot=Join-Path $case 'native';[IO.Directory]::CreateDirectory($nativeRoot)|Out-Null
                foreach($item in Get-VVChildItem -LiteralPath (Join-Path $local 'artifacts') -Recurse -File){
                    $relative=Get-VVRelative (Join-Path $local 'artifacts') $item.FullName
                    $destination=Join-Path $nativeRoot $relative
                    [IO.Directory]::CreateDirectory((Split-Path -Parent $destination))|Out-Null
                    [IO.File]::Copy($item.FullName,$destination,$true)
                }
            }
        }
    } $Spec
}
